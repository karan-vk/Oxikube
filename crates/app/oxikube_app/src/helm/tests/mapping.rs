//! Manifest text -> `ResourceRef`s: namespace defaulting, scopes from discovery, hooks, errors.

use oxikube_domain::helm::parse_manifest;
use oxikube_domain::ids::{Gvk, Scope};

use super::{cluster, release, served};
use crate::helm::{KindScopes, ResourcePlan};

const MANIFEST: &str = r#"---
# Source: web/templates/sa.yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: web
---
# Source: web/templates/deploy.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  namespace: other
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: web
  namespace: ignored
---
apiVersion: example.com/v1
kind: Widget
metadata:
  name: w
---
apiVersion: batch/v1
kind: Job
metadata:
  name: migrate
  annotations:
    helm.sh/hook: pre-upgrade
---
key: [broken
"#;

fn planned(kinds: &KindScopes) -> ResourcePlan {
    ResourcePlan::build(&release(), &parse_manifest(MANIFEST), kinds)
}

#[test]
fn namespaced_kinds_default_to_the_release_namespace() {
    let plan = planned(&served());
    let sa = &plan.resources[0];
    assert_eq!(sa.reference.namespace(), Some("apps"));
    assert_eq!(sa.reference.gvk, Gvk::new("", "v1", "ServiceAccount"));
    assert_eq!(sa.reference.cluster, cluster());
    assert_eq!(sa.scope, Some(Scope::Namespaced));
    assert_eq!(sa.source.as_deref(), Some("web/templates/sa.yaml"));
}

#[test]
fn an_explicit_manifest_namespace_wins() {
    let plan = planned(&served());
    assert_eq!(plan.resources[1].reference.namespace(), Some("other"));
}

#[test]
fn cluster_scoped_kinds_have_no_namespace() {
    let plan = planned(&served());
    let role = &plan.resources[2];
    assert_eq!(role.reference.namespace(), None);
    assert_eq!(role.scope, Some(Scope::Cluster));
}

#[test]
fn unserved_kinds_are_kept_with_unknown_scope() {
    let plan = planned(&served());
    let widget = &plan.resources[3];
    assert_eq!(widget.scope, None);
    assert_eq!(
        widget.reference.gvk,
        Gvk::new("example.com", "v1", "Widget")
    );
    // Without discovery everything is unknown: no hard-coded table.
    let none = KindScopes::default();
    assert!(planned(&none).resources.iter().all(|r| r.scope.is_none()));
}

#[test]
fn hooks_and_broken_documents_are_apart() {
    let plan = planned(&served());
    assert_eq!(plan.resources.len(), 4);
    assert_eq!(plan.hooks.len(), 1);
    assert_eq!(plan.hooks[0].name, "migrate");
    assert_eq!(plan.errors.len(), 1);
}

#[test]
fn the_served_version_is_used_when_the_manifest_names_an_old_one() {
    let plan = ResourcePlan::build(
        &release(),
        &parse_manifest("apiVersion: apps/v1beta1\nkind: Deployment\nmetadata:\n  name: old\n"),
        &served(),
    );
    assert_eq!(
        plan.resources[0].reference.gvk,
        Gvk::new("apps", "v1", "Deployment")
    );
}
