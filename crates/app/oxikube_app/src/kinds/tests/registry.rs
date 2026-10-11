//! Registration, duplicate rejection, lookup by any version, the generic fallback, actions and
//! templates.

use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::Capabilities;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};

use super::{built_ins, meta};
use crate::columns::{Cell, Column, ColumnId, ColumnProvider};
use crate::kinds::{GroupKind, KindMeta, KindRegistry, RegisterKindError, Section};
use crate::store::StoreObject;
use crate::templates::{Template, TemplateCatalog};

fn registry() -> KindRegistry {
    let mut registry = KindRegistry::new();
    for meta in built_ins() {
        registry.register(meta).expect("registers");
    }
    registry
}

#[test]
fn a_registered_kind_is_found_at_any_version() {
    let registry = registry();
    assert_eq!(registry.len(), built_ins().len());
    let v1 = registry
        .get(&Gvk::new("apps", "v1", "Deployment"))
        .expect("registered");
    assert_eq!(&*v1.plural, "deployments");
    // Another version of the same kind is the same spec.
    let beta = registry
        .get(&Gvk::new("apps", "v1beta2", "Deployment"))
        .expect("any version");
    assert!(Arc::ptr_eq(v1, beta));
    // Same kind name, other group: not the same kind.
    assert!(
        registry
            .get(&Gvk::new("example.io", "v1", "Deployment"))
            .is_none()
    );
}

#[test]
fn a_duplicate_kind_is_refused_and_the_first_stays() {
    let mut registry = registry();
    let again = meta("apps", "Deployment", "deployments", Section::CONFIG, 1);
    let error = registry.register(again).expect_err("duplicate");
    assert_eq!(
        error,
        RegisterKindError::Duplicate(GroupKind::of(&Gvk::new("apps", "v1", "Deployment")))
    );
    assert_eq!(error.to_string(), "apps/Deployment is registered twice");
    // Registering at another version is the same kind, refused too.
    let other_version = KindMeta::new(
        Gvk::new("apps", "v1beta1", "Deployment"),
        "deployments",
        "Deployments",
        Scope::Namespaced,
    );
    assert!(registry.register(other_version).is_err());
    let kept = registry.kind_for(&Gvk::new("apps", "v1", "Deployment"));
    assert_eq!(kept.sidebar.map(|s| s.section), Some(Section::WORKLOADS));
    assert_eq!(registry.len(), built_ins().len());
}

#[test]
fn an_unknown_kind_falls_back_to_the_generic_spec() {
    let registry = registry();
    let widget = Gvk::new("example.io", "v1", "Widget");
    assert!(!registry.is_registered(&widget));
    let spec = registry.kind_for(&widget);
    assert!(spec.is_generic());
    assert!(
        spec.columns.provider().is_none(),
        "the shared (E07) columns"
    );
    assert!(spec.actions.is_empty());
    assert!(spec.sidebar.is_none());
    // The empty registry answers too.
    assert!(KindRegistry::new().kind_for(&widget).is_generic());
}

#[test]
fn the_generic_spec_cannot_be_registered() {
    let mut registry = KindRegistry::new();
    assert_eq!(
        registry.register(KindMeta::generic()),
        Err(RegisterKindError::Unnamed)
    );
    let no_plural = KindMeta::new(Gvk::new("", "v1", "Thing"), "", "Things", Scope::Namespaced);
    assert_eq!(
        registry.register(no_plural),
        Err(RegisterKindError::Unnamed)
    );
}

#[test]
fn actions_are_declared_commands() {
    let mut registry = KindRegistry::new();
    let pods = meta("", "Pod", "pods", Section::WORKLOADS, 10)
        .with_actions([CommandId::POD_SHELL, CommandId::POD_VIEW_LOGS]);
    registry.register(pods).expect("declared commands");
    let pod = registry.kind_for(&Gvk::new("", "v1", "Pod"));
    assert!(pod.offers(CommandId::POD_SHELL));
    assert!(!pod.offers(CommandId::NODE_SHELL));
    let offering: Vec<_> = registry
        .kinds_offering(CommandId::POD_SHELL)
        .map(|m| m.plural.to_string())
        .collect();
    assert_eq!(offering, ["pods"]);

    let bogus = CommandId::new("kind::DoesNotExist");
    let services = meta("", "Service", "services", Section::NETWORK, 10).with_actions([bogus]);
    let error = registry.register(services).expect_err("undeclared");
    assert!(matches!(error, RegisterKindError::UndeclaredAction { action, .. } if action == bogus));
    assert!(!registry.is_registered(&Gvk::new("", "v1", "Service")));
}

/// A provider with one column, to tell it from the shared one.
struct OneColumn;

impl ColumnProvider for OneColumn {
    fn columns(&self, _: &Gvk, _: Capabilities) -> Arc<[Column]> {
        Arc::new([])
    }

    fn cell<'a>(&self, _: &'a StoreObject, _: &ColumnId, _: Timestamp) -> Cell<'a> {
        Cell::empty()
    }
}

#[test]
fn a_kind_can_bring_its_own_columns() {
    let mut registry = KindRegistry::new();
    let provider: Arc<dyn ColumnProvider> = Arc::new(OneColumn);
    registry
        .register(meta("", "Pod", "pods", Section::WORKLOADS, 10).with_columns(provider.clone()))
        .expect("registers");
    let pod = registry.kind_for(&Gvk::new("", "v1", "Pod"));
    let own = pod.columns.provider().expect("its own provider");
    assert!(Arc::ptr_eq(own, &provider));
}

fn catalog() -> TemplateCatalog {
    TemplateCatalog::of(vec![
        Template::embedded(
            "Deployment",
            "Replicated pods",
            "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n",
        ),
        Template::embedded(
            "Pod",
            "One pod",
            "apiVersion: v1\nkind: Pod\nmetadata:\n  name: one\n",
        ),
        Template::embedded(
            "Deployment and Service",
            "Two documents",
            "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n",
        ),
    ])
}

#[test]
fn templates_are_found_by_kind_or_named() {
    let mut registry = registry();
    let catalog = catalog();
    let names = |templates: Vec<&Template>| -> Vec<String> {
        templates.iter().map(|t| t.name.to_string()).collect()
    };
    // By kind: every template whose first document is a Deployment.
    let deployments = registry.templates_for(&Gvk::new("apps", "v1", "Deployment"), &catalog);
    assert_eq!(names(deployments), ["Deployment", "Deployment and Service"]);
    // Named: in the spec's order, a name the catalog lacks skipped.
    let services = meta("", "Endpoints", "endpoints", Section::NETWORK, 20)
        .with_templates(["Deployment and Service", "Missing"]);
    registry.register(services).expect("registers");
    let named = registry.templates_for(&Gvk::new("", "v1", "Endpoints"), &catalog);
    assert_eq!(names(named), ["Deployment and Service"]);
    // An unregistered kind gets none.
    assert!(
        registry
            .templates_for(&Gvk::new("example.io", "v1", "Widget"), &catalog)
            .is_empty()
    );
}
