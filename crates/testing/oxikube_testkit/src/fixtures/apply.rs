//! The apply fixture (E20-S10): a directory of 50 manifests for the bulk apply tests.
//!
//! It lives on disk at `fixtures/apply/` (not embedded: tests also point `kubectl` and the
//! manifest loader at it) and is spread over subdirectories whose names sort **opposite** to the
//! right apply order: the custom resources are in `00-custom-resources/`, the Namespaces in
//! `99-namespaces/`, the CustomResourceDefinition in `98-crd/`. Applying in file order fails;
//! applying in kind order works, which is what the tests prove.
//!
//! | Directory | Objects |
//! |---|---|
//! | `00-custom-resources/` | 5 `Gadget`s (YAML and JSON files) |
//! | `05-networking/` | 2 Ingresses, 1 NetworkPolicy, 2 PodDisruptionBudgets |
//! | `10-batch/` | a Job and 2 CronJobs (suspended) |
//! | `20-workloads/` | 5 Deployments (no replicas), a StatefulSet (no replicas) |
//! | `30-services/` | 4 Services |
//! | `40-config/` | 9 ConfigMaps, 4 Secrets (dummy values) |
//! | `50-limits/` | a ResourceQuota, a LimitRange |
//! | `60-rbac/` | 3 ServiceAccounts, 2 Roles, 2 RoleBindings, a ClusterRole and its binding |
//! | `98-crd/` | the `Gadget` CustomResourceDefinition |
//! | `99-namespaces/` | 2 Namespaces (and a `NOTES.txt` the loader must skip) |
//!
//! Every name that must be unique per run (the two namespaces, the CRD group, the ClusterRole)
//! contains [`RUN_TOKEN`]; the functions here replace it with a run id, so tests on a shared
//! cluster never meet. Files and documents are read in path order, like the app's loader.
//! Secrets hold obvious dummy data only.

use std::io;
use std::path::{Path, PathBuf};

use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::manifests::{ManifestDoc, SourceId, parse_documents};

use crate::kinds::kind;

/// The word in the fixture's names that stands for the run id.
pub const RUN_TOKEN: &str = "fxrun";

/// How many objects the directory holds.
pub const MANIFEST_COUNT: usize = 50;

/// One manifest file of the fixture, its run token already replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureFile {
    /// The path below the fixture directory, `/`-separated.
    pub path: String,
    /// The text.
    pub text: String,
}

/// The fixture directory on disk.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/apply")
}

fn is_manifest(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| ["yaml", "yml", "json"].contains(&extension))
}

fn collect(root: &Path, at: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(at)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else if is_manifest(&path) {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
    Ok(())
}

/// The manifest files in path order (`.yaml`, `.yml`, `.json`; the rest is skipped), with `run`
/// in place of [`RUN_TOKEN`].
///
/// # Panics
///
/// When the fixture directory cannot be read (a broken checkout).
pub fn files(run: &str) -> Vec<FixtureFile> {
    let root = dir();
    let mut relative = Vec::new();
    collect(&root, &root, &mut relative).expect("the apply fixture directory is readable");
    relative
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(root.join(&path))
                .expect("a fixture file is readable")
                .replace(RUN_TOKEN, run);
            FixtureFile {
                path: path.to_string_lossy().replace('\\', "/"),
                text,
            }
        })
        .collect()
}

/// Every object of the fixture in file order (the wrong apply order), each remembering its file
/// and line.
///
/// # Panics
///
/// When a fixture file does not parse: the fixture is broken.
pub fn docs(run: &str) -> Vec<ManifestDoc> {
    files(run)
        .into_iter()
        .flat_map(|file| {
            let (docs, errors) = parse_documents(&file.text, SourceId::new(file.path.as_str()));
            assert!(errors.is_empty(), "{}: {errors:?}", file.path);
            docs
        })
        .collect()
}

/// Writes the fixture (run token replaced) below `dest`, keeping its layout, for a test that
/// loads a directory.
///
/// # Errors
///
/// The file system's.
pub fn write_to(dest: &Path, run: &str) -> io::Result<()> {
    for file in files(run) {
        let path = dest.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, file.text)?;
    }
    Ok(())
}

/// The two namespaces the fixture makes: `<run>-app` and `<run>-data`.
pub fn namespaces(run: &str) -> [String; 2] {
    [format!("{run}-app"), format!("{run}-data")]
}

/// The CustomResourceDefinition's group.
pub fn crd_group(run: &str) -> String {
    format!("{run}.apply.test.oxikube.dev")
}

/// The objects a run leaves outside its two namespaces, as `(kubectl resource, name)`: delete
/// these and the namespaces to clean up.
pub fn cluster_scoped(run: &str) -> Vec<(&'static str, String)> {
    vec![
        ("crd", format!("gadgets.{}", crd_group(run))),
        ("clusterrolebinding", format!("{run}-node-reader")),
        ("clusterrole", format!("{run}-node-reader")),
    ]
}

/// The kinds of the fixture as a cluster serves them (the core set is in
/// [`kinds::core_kinds`](crate::kinds::core_kinds)): load them into `FakeDiscoveryPort` beside
/// those so a plan knows which kinds are namespaced.
pub fn served_kinds(run: &str) -> Vec<ResourceKind> {
    let rbac = "rbac.authorization.k8s.io";
    vec![
        kind(&crd_group(run), "v1", "Gadget", "gadgets").build(),
        kind(
            "apiextensions.k8s.io",
            "v1",
            "CustomResourceDefinition",
            "customresourcedefinitions",
        )
        .cluster_scoped()
        .build(),
        kind("networking.k8s.io", "v1", "Ingress", "ingresses").build(),
        kind(
            "networking.k8s.io",
            "v1",
            "NetworkPolicy",
            "networkpolicies",
        )
        .build(),
        kind(
            "policy",
            "v1",
            "PodDisruptionBudget",
            "poddisruptionbudgets",
        )
        .build(),
        kind("batch", "v1", "Job", "jobs").build(),
        kind("batch", "v1", "CronJob", "cronjobs").build(),
        kind("", "v1", "ResourceQuota", "resourcequotas").build(),
        kind("", "v1", "LimitRange", "limitranges").build(),
        kind(rbac, "v1", "Role", "roles").build(),
        kind(rbac, "v1", "RoleBinding", "rolebindings").build(),
        kind(rbac, "v1", "ClusterRole", "clusterroles")
            .cluster_scoped()
            .build(),
        kind(rbac, "v1", "ClusterRoleBinding", "clusterrolebindings")
            .cluster_scoped()
            .build(),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn kind_counts(run: &str) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for doc in docs(run) {
            let kind = doc.json["kind"].as_str().unwrap().to_owned();
            *counts.entry(kind).or_insert(0) += 1;
        }
        counts
    }

    #[test]
    fn it_has_fifty_objects_and_skips_what_is_not_a_manifest() {
        assert_eq!(docs("r1").len(), MANIFEST_COUNT);
        let files = files("r1");
        assert!(files.iter().all(|f| !f.path.ends_with(".txt")));
        assert!(files.iter().any(|f| f.path.ends_with(".json")));
        assert!(
            files.iter().any(|f| f.path.contains("/pdb/")),
            "subdirectories"
        );
        let paths: Vec<_> = files.iter().map(|f| f.path.clone()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted, "path order");
    }

    #[test]
    fn it_covers_the_kinds_of_the_plan() {
        let counts = kind_counts("r1");
        for (kind, at_least) in [
            ("Namespace", 2),
            ("CustomResourceDefinition", 1),
            ("Gadget", 5),
            ("ServiceAccount", 3),
            ("Role", 2),
            ("RoleBinding", 2),
            ("ClusterRole", 1),
            ("ClusterRoleBinding", 1),
            ("ConfigMap", 9),
            ("Secret", 4),
            ("Service", 4),
            ("Deployment", 5),
            ("StatefulSet", 1),
            ("Job", 1),
            ("CronJob", 2),
            ("Ingress", 2),
        ] {
            assert!(counts.get(kind).copied().unwrap_or(0) >= at_least, "{kind}");
        }
    }

    #[test]
    fn file_order_is_the_opposite_of_apply_order() {
        let docs = docs("r1");
        let position = |kind: &str| docs.iter().position(|d| d.json["kind"] == kind).unwrap();
        assert!(position("Gadget") < position("CustomResourceDefinition"));
        assert!(position("CustomResourceDefinition") < position("Namespace"));
        assert!(position("Deployment") < position("Namespace"));
    }

    #[test]
    fn the_run_id_replaces_the_token_everywhere() {
        let text: String = files("oxi-test-abc").into_iter().map(|f| f.text).collect();
        assert!(text.contains("oxi-test-abc-app"));
        assert!(text.contains("gadgets.oxi-test-abc.apply.test.oxikube.dev"));
        assert!(!text.contains(RUN_TOKEN));
    }

    #[test]
    fn secrets_hold_dummy_values_only() {
        for doc in docs("r1").iter().filter(|d| d.json["kind"] == "Secret") {
            for value in doc.json["stringData"].as_object().unwrap().values() {
                let value = value.as_str().unwrap();
                assert!(
                    value.contains("dummy") || value.contains("not-a-real"),
                    "{value}"
                );
            }
        }
    }

    #[test]
    fn cluster_scoped_names_match_the_objects() {
        let docs = docs("r1");
        for (_, name) in cluster_scoped("r1") {
            assert!(
                docs.iter()
                    .any(|d| d.json["metadata"]["name"] == name.as_str()),
                "{name}"
            );
        }
    }
}
