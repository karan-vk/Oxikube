//! Registration through `init(cx)`: the built-in kinds, duplicate rejection, the generic
//! fallback, and the sections published to the sidebar registry.

use std::time::Instant;

use gpui::TestAppContext;
use oxikube_app::{KindMeta, RegisterKindError, Section};
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_workspace::sidebar::{SectionBody, SidebarRegistry, SidebarSection, SidebarTarget};
use oxikube_workspace::test_support::open_workspace;

use super::pods_with_own_columns;
use crate::kinds::{DetailRenderer, KindSpec, KindSpecs};

/// The entries' ids of the sidebar section `id`.
fn entry_ids(sections: &[SidebarSection], id: &str) -> Vec<String> {
    let section = sections
        .iter()
        .find(|s| s.id.as_ref() == id)
        .unwrap_or_else(|| panic!("section {id}"));
    match &section.body {
        SectionBody::Entries(entries) => entries.iter().map(|e| e.id.to_string()).collect(),
        SectionBody::CustomResources => panic!("{id} holds entries"),
    }
}

#[gpui::test]
fn init_registers_the_built_in_kinds_and_fills_the_sidebar(cx: &mut TestAppContext) {
    let (_ws, mut vcx) = open_workspace(cx);
    let started = Instant::now();
    vcx.update(|_, cx| {
        oxikube_workspace::sidebar::init(cx);
        crate::kinds::init(cx);
    });
    let elapsed = started.elapsed();
    let (registry, sections) =
        vcx.update(|_, cx| (KindSpecs::registry(cx), SidebarRegistry::sections(cx)));
    assert!(registry.len() >= 40, "{} kinds", registry.len());
    // Generous for a debug build; the release cost is reported in the PR.
    assert!(elapsed.as_millis() < 250, "registering took {elapsed:?}");

    // Freelens' order of the Workloads section, from the registry, after the Workloads overview
    // page (E13-S09: the "Cluster" header is the cluster overview).
    assert_eq!(
        entry_ids(&sections, "workloads"),
        [
            "workloads-overview",
            "pods",
            "deployments",
            "daemonsets",
            "statefulsets",
            "replicasets",
            "replicationcontrollers",
            "jobs",
            "cronjobs"
        ]
    );
    assert_eq!(entry_ids(&sections, "nodes"), ["nodes"]);
    assert_eq!(entry_ids(&sections, "namespaces"), ["namespaces"]);
    assert!(entry_ids(&sections, "network").contains(&"gateways".to_owned()));
    assert!(entry_ids(&sections, "storage").contains(&"volumesnapshots".to_owned()));
    // Each entry names its kind, for the per-cluster gate, and lists its plural.
    let workloads = sections
        .iter()
        .find(|s| s.id.as_ref() == "workloads")
        .unwrap();
    let SectionBody::Entries(entries) = &workloads.body else {
        panic!("entries");
    };
    let deployments = entries
        .iter()
        .find(|e| e.id.as_ref() == "deployments")
        .unwrap();
    let kind = deployments.kind.as_ref().expect("gated on its kind");
    assert_eq!(kind.gvk, Gvk::new("apps", "v1", "Deployment"));
    assert_eq!(kind.scope, Scope::Namespaced);
    assert_eq!(
        deployments.target,
        Some(SidebarTarget::kind("apps", "deployments"))
    );
    // The workspace's own sections are still there, in their order (Events, Helm, Access
    // Control between Namespaces and Custom Resources).
    let ids: Vec<&str> = sections.iter().map(|s| s.id.as_ref()).collect();
    assert_eq!(
        ids,
        [
            "cluster",
            "nodes",
            "workloads",
            "config",
            "network",
            "storage",
            "namespaces",
            "events",
            "helm",
            "access-control",
            "custom-resources"
        ]
    );

    // The kinds E08 and E09 ship actions for list them.
    let pod = registry.kind_for(&Gvk::new("", "v1", "Pod"));
    assert!(pod.offers(CommandId::POD_SHELL) && pod.offers(CommandId::POD_VIEW_LOGS));
    assert!(
        registry
            .kind_for(&Gvk::new("", "v1", "Node"))
            .offers(CommandId::NODE_SHELL)
    );
    let crd = registry.kind_for(&Gvk::new(
        "apiextensions.k8s.io",
        "v1",
        "CustomResourceDefinition",
    ));
    assert!(
        crd.sidebar.is_none(),
        "listed by Custom Resources' Definitions entry"
    );
}

#[gpui::test]
fn a_duplicate_registration_is_refused_and_the_first_spec_stays(cx: &mut TestAppContext) {
    let (_ws, mut vcx) = open_workspace(cx);
    vcx.update(|_, cx| {
        oxikube_workspace::sidebar::init(cx);
        KindSpecs::register(cx, KindSpec::new(pods_with_own_columns())).expect("first");
        let again = KindMeta::new(Gvk::new("", "v1", "Pod"), "pods", "Pods", Scope::Namespaced)
            .in_section(Section::CONFIG, 1);
        let error = KindSpecs::register(cx, KindSpec::new(again)).expect_err("duplicate");
        assert!(matches!(error, RegisterKindError::Duplicate(_)));
        // The built-ins register around it: Pods keeps its own columns.
        crate::kinds::init(cx);
        assert!(KindSpecs::columns_for(cx, &Gvk::new("", "v1", "Pod")).is_some());
        assert!(KindSpecs::columns_for(cx, &Gvk::new("apps", "v1", "Deployment")).is_none());
        let config = SidebarRegistry::sections(cx);
        assert!(!entry_ids(&config, "config").contains(&"pods".to_owned()));
    });
}

#[gpui::test]
fn an_unknown_kind_gets_the_generic_spec(cx: &mut TestAppContext) {
    let (_ws, mut vcx) = open_workspace(cx);
    vcx.update(|_, cx| {
        let widget = Gvk::new("example.io", "v1", "Widget");
        // Before anything registered, and after.
        assert!(KindSpecs::kind_for(cx, &widget).is_generic());
        oxikube_workspace::sidebar::init(cx);
        crate::kinds::init(cx);
        assert!(KindSpecs::kind_for(cx, &widget).is_generic());
        assert!(KindSpecs::columns_for(cx, &widget).is_none());
        assert!(KindSpecs::detail_for(cx, &widget).is_generic());
        assert!(!KindSpecs::kind_for(cx, &Gvk::new("", "v1", "Pod")).is_generic());
    });
}

#[gpui::test]
fn a_kind_in_a_new_section_adds_the_section_at_its_rank(cx: &mut TestAppContext) {
    let (_ws, mut vcx) = open_workspace(cx);
    vcx.update(|_, cx| {
        oxikube_workspace::sidebar::init(cx);
        let section = Section::new("mesh", "Service Mesh", 550);
        let meta = KindMeta::new(
            Gvk::new("mesh.io", "v1", "Route"),
            "routes",
            "Routes",
            Scope::Namespaced,
        )
        .in_section(section, 10);
        KindSpecs::register(cx, KindSpec::new(meta).with_detail(DetailRenderer::Generic))
            .expect("registers");
        let sections = SidebarRegistry::sections(cx);
        let ids: Vec<&str> = sections.iter().map(|s| s.id.as_ref()).collect();
        let mesh = ids.iter().position(|id| *id == "mesh").expect("added");
        assert_eq!(ids[mesh - 1], "network");
        assert_eq!(ids[mesh + 1], "storage");
        assert_eq!(entry_ids(&sections, "mesh"), ["routes"]);
    });
}
