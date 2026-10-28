//! Micro-benchmark (E17 audit): the `SubjectIndex` queries behind the RBAC Users / Groups matrix,
//! over a policy the size Crossplane-style providers give a real cluster.
//!
//! The fixture holds 12 403 RBAC objects: 1 503 ClusterRoles (1 500 provider roles, each labelled
//! into one of three aggregated ClusterRoles that collect 500 apiece), 2 400 Roles (8 in each of
//! 300 namespaces), 7 500 RoleBindings (25 per namespace) and 1 000 ClusterRoleBindings. An SSO
//! group is bound in every namespace and by 25 ClusterRoleBindings: 325 grants, 26 of them in
//! `ns-7`.
//!
//! Prints min / median / p95 of what a page refresh asks:
//!
//! * `grants_for` the group (every scope), `rules_in` one namespace;
//! * `scopes_of`, the cluster matrix, a namespace matrix and one cell's provenance;
//! * one Subject page refresh as `SubjectsView` runs it (`scopes_of`, then the matrix of the
//!   scope it lands on), with a warm role cache;
//! * the heap one index holds.
//!
//! `cargo bench -p oxikube_app --bench rbac_matrix`. Under `cargo test --all-targets` (no
//! `--bench` flag) it runs one small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use oxikube_app::access::SubjectIndex;
use oxikube_app::rbac_subjects::{Inherit, MatrixColumn, provenance, scopes_of, subject_matrix};
use oxikube_app::store::{RowChange, StoreObject};
use oxikube_domain::Resource;
use oxikube_domain::kinds::Verb;
use oxikube_domain::rbac::{GrantScope, RbacKind, Subject};
use serde_json::{Value, json};

const NAMESPACES: usize = 300;
const PROVIDERS: usize = 1_500;
const ROLES_PER_NAMESPACE: usize = 8;
const BINDINGS_PER_NAMESPACE: usize = 25;
const CLUSTER_BINDINGS: usize = 1_000;
const GROUP_CLUSTER_BINDINGS: usize = 25;
const GROUP: &str = "sso:platform-devs";
const LEVELS: [&str; 3] = ["view", "edit", "admin"];

/// Counts the bytes the process holds, to report what the index costs.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: forwards to the system allocator and only counts.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract for `layout`; passed through unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `ptr` came from `System.alloc` above with this same `layout`, per the caller's contract.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn object(value: Value) -> Arc<StoreObject> {
    Arc::new(StoreObject::Resource(
        Resource::from_json(value).expect("a valid RBAC object"),
    ))
}

fn cluster_roles() -> Vec<Arc<StoreObject>> {
    let mut roles: Vec<Arc<StoreObject>> = (0..PROVIDERS)
        .map(|i| {
            let level = LEVELS[i % LEVELS.len()];
            let verbs = match level {
                "view" => json!(["get", "list", "watch"]),
                "edit" => json!([
                    "get", "list", "watch", "create", "update", "patch", "delete"
                ]),
                _ => json!(["*"]),
            };
            object(json!({
                "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
                "metadata": {"name": format!("provider-{i}"),
                             "labels": {format!("rbac.example.io/aggregate-to-{level}"): "true"}},
                "rules": [
                    {"apiGroups": [format!("p{i}.example.io")],
                     "resources": [format!("things{i}"), format!("widgets{i}")], "verbs": verbs},
                    {"apiGroups": [format!("p{i}.example.io")],
                     "resources": [format!("things{i}/status")], "verbs": ["get", "patch"]},
                ],
            }))
        })
        .collect();
    for level in LEVELS {
        roles.push(object(json!({
            "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
            "metadata": {"name": format!("aggregate-{level}")},
            "aggregationRule": {"clusterRoleSelectors": [
                {"matchLabels": {format!("rbac.example.io/aggregate-to-{level}"): "true"}}
            ]},
            "rules": [],
        })));
    }
    roles
}

fn roles() -> Vec<Arc<StoreObject>> {
    (0..NAMESPACES)
        .flat_map(|ns| {
            (0..ROLES_PER_NAMESPACE).map(move |j| {
                object(json!({
                    "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
                    "metadata": {"name": format!("role-{j}"), "namespace": format!("ns-{ns}")},
                    "rules": [{"apiGroups": [""], "resources": ["configmaps", "secrets"],
                               "verbs": ["get", "list"], "resourceNames": [format!("app-{j}")]},
                              {"apiGroups": ["apps"], "resources": ["deployments"],
                               "verbs": ["get", "list", "watch", "patch"]}],
                }))
            })
        })
        .collect()
}

fn role_bindings() -> Vec<Arc<StoreObject>> {
    (0..NAMESPACES)
        .flat_map(|ns| {
            (0..BINDINGS_PER_NAMESPACE).map(move |k| {
                let (role, subject) = if k == 0 {
                    (
                        json!({"kind": "ClusterRole", "name": "aggregate-view"}),
                        json!({"kind": "Group", "name": GROUP}),
                    )
                } else {
                    (
                        json!({"kind": "Role", "name": format!("role-{}", k % ROLES_PER_NAMESPACE)}),
                        json!({"kind": "User", "name": format!("user-{}", ns * BINDINGS_PER_NAMESPACE + k)}),
                    )
                };
                let mut role = role;
                role["apiGroup"] = json!("rbac.authorization.k8s.io");
                object(json!({
                    "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "RoleBinding",
                    "metadata": {"name": format!("rb-{k}"), "namespace": format!("ns-{ns}")},
                    "roleRef": role,
                    "subjects": [subject],
                }))
            })
        })
        .collect()
}

fn cluster_role_bindings() -> Vec<Arc<StoreObject>> {
    (0..CLUSTER_BINDINGS)
        .map(|k| {
            let (role, subjects) = if k < GROUP_CLUSTER_BINDINGS {
                let role = if k == 0 {
                    "aggregate-edit".to_owned()
                } else {
                    format!("provider-{}", k * 37)
                };
                (role, json!([{"kind": "Group", "name": GROUP}]))
            } else {
                (
                    format!("provider-{}", k % PROVIDERS),
                    json!([{"kind": "User", "name": format!("cluster-user-{}", 2 * k)},
                           {"kind": "User", "name": format!("cluster-user-{}", 2 * k + 1)}]),
                )
            };
            object(json!({
                "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRoleBinding",
                "metadata": {"name": format!("crb-{k}")},
                "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": role},
                "subjects": subjects,
            }))
        })
        .collect()
}

fn report(label: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<46} min {:>9.1?}  median {:>9.1?}  p95 {:>9.1?}",
        at(0.0),
        at(0.5),
        at(0.95)
    );
}

fn time<R>(samples: usize, mut run: impl FnMut() -> R) -> Vec<Duration> {
    (0..samples)
        .map(|_| {
            let start = Instant::now();
            black_box(run());
            start.elapsed()
        })
        .collect()
}

/// One Subject page refresh, as `SubjectsView` computes it when no scope is picked: the scopes,
/// then the matrix of the scope the page lands on (cluster-wide when there are cluster grants).
fn refresh(index: &SubjectIndex, subject: &Subject) -> usize {
    let scopes = scopes_of(index, subject, Inherit::ImplicitGroups);
    let matrix = subject_matrix(
        index,
        subject,
        &GrantScope::Cluster,
        Inherit::ImplicitGroups,
    );
    scopes.len() + matrix.lines.len()
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let samples = if smoke { 1 } else { 30 };
    let feeds = [
        (RbacKind::ClusterRole, cluster_roles()),
        (RbacKind::Role, roles()),
        (RbacKind::RoleBinding, role_bindings()),
        (RbacKind::ClusterRoleBinding, cluster_role_bindings()),
    ];
    let objects: usize = feeds.iter().map(|(_, rows)| rows.len()).sum();
    println!("SubjectIndex over {objects} RBAC objects ({samples} samples)");

    let before = LIVE.load(Ordering::Relaxed);
    let mut index = SubjectIndex::new();
    for (kind, rows) in &feeds {
        index.apply_rows(*kind, &RowChange::Snapshot(rows.clone()));
    }
    let group = Subject::Group(GROUP.into());
    // Warm the role cache the way the first refresh does.
    black_box(refresh(&index, &group));
    let held = LIVE.load(Ordering::Relaxed).saturating_sub(before);
    println!(
        "heap held by one index (role cache warm): {:.1} MiB",
        held as f64 / 1_048_576.0
    );
    let grants = index.grants_for(&group);
    let in_ns7 = grants
        .iter()
        .filter(|grant| grant.scope.covers(Some("ns-7")))
        .count();
    println!("{GROUP}: {} grants, {in_ns7} in ns-7", grants.len());
    drop(grants);

    let ns7 = GrantScope::Namespace("ns-7".into());
    report(
        "grants_for (every scope)",
        time(samples, || index.grants_for(&group).len()),
    );
    report(
        "rules_in(ns-7)",
        time(samples, || {
            index.rules_in(&group, Some("ns-7")).rules().len()
        }),
    );
    report(
        "scopes_of",
        time(samples, || {
            scopes_of(&index, &group, Inherit::ImplicitGroups).len()
        }),
    );
    report(
        "subject_matrix(cluster)",
        time(samples, || {
            subject_matrix(
                &index,
                &group,
                &GrantScope::Cluster,
                Inherit::ImplicitGroups,
            )
            .lines
            .len()
        }),
    );
    report(
        "subject_matrix(ns-7)",
        time(samples, || {
            subject_matrix(&index, &group, &ns7, Inherit::ImplicitGroups)
                .lines
                .len()
        }),
    );
    let matrix = subject_matrix(&index, &group, &ns7, Inherit::ImplicitGroups);
    let line = matrix.lines.first().cloned().expect("the group has rules");
    report(
        "provenance(ns-7, first line, GET)",
        time(samples, || {
            provenance(
                &index,
                &group,
                &ns7,
                Inherit::ImplicitGroups,
                &line,
                MatrixColumn::Verb(Verb::Get),
            )
            .len()
        }),
    );
    report(
        "page refresh (scopes + landing matrix)",
        time(samples, || refresh(&index, &group)),
    );
}
