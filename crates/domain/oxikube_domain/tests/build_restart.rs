//! `restart_patch` (the kdash rollout-restart builder) and `set_image_patch`, pinned as JSON, and
//! the licence check for the copied builder: its file header keeps the kdash MIT notice and
//! `THIRD_PARTY_NOTICES.md` lists the file.

use jiff::Timestamp;
use oxikube_domain::build::{
    RESTARTED_AT_ANNOTATION, plan_image_changes, restart_patch, set_image_patch, template_images,
};
use oxikube_domain::command::ImageChange;
use oxikube_domain::{ErrorKind, Resource};
use serde_json::json;

const RESTART_SOURCE: &str = include_str!("../src/build/restart.rs");
const NOTICES: &str = include_str!("../../../../THIRD_PARTY_NOTICES.md");

#[test]
fn the_restart_patch_is_exactly_the_kubectl_annotation_at_the_given_second() {
    let at: Timestamp = "2026-10-06T12:34:56.9Z".parse().unwrap();
    assert_eq!(
        restart_patch(at),
        json!({"spec": {"template": {"metadata": {"annotations": {
            "kubectl.kubernetes.io/restartedAt": "2026-10-06T12:34:56Z"
        }}}}})
    );
    assert_eq!(RESTARTED_AT_ANNOTATION, "kubectl.kubernetes.io/restartedAt");
    // Another instant, another value: a second restart always changes the template.
    let later: Timestamp = "2026-10-06T12:35:00Z".parse().unwrap();
    assert_ne!(restart_patch(at), restart_patch(later));
}

#[test]
fn the_copied_builder_carries_the_kdash_licence_header_and_a_notices_entry() {
    let header: String = RESTART_SOURCE
        .lines()
        .take_while(|line| line.starts_with("//") && !line.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");
    for needle in [
        "kdash-rs/kdash",
        "Copyright (c) 2021 Deepu K Sasidharan",
        "Permission is hereby granted, free of charge",
        "THE SOFTWARE IS PROVIDED \"AS IS\"",
    ] {
        assert!(
            header.contains(needle),
            "restart.rs header lacks {needle:?}"
        );
    }
    assert!(
        NOTICES.contains("crates/domain/oxikube_domain/src/build/restart.rs"),
        "THIRD_PARTY_NOTICES.md lists the file"
    );
}

#[test]
fn the_image_patch_names_only_the_changed_containers() {
    let workload = Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "StatefulSet",
        "metadata": {"name": "db", "namespace": "shop"},
        "spec": {"template": {"spec": {
            "initContainers": [{"name": "init", "image": "busybox:1"}],
            "containers": [
                {"name": "db", "image": "pg:15"},
                {"name": "exporter", "image": "exp:1"}
            ],
        }}},
    }))
    .unwrap();
    let slots = template_images(&workload).unwrap();
    let edits = plan_image_changes(
        &slots,
        &[
            ImageChange::new("db", "pg:16"),
            ImageChange::new("init", "busybox:2"),
        ],
    )
    .unwrap();
    assert_eq!(
        set_image_patch(&edits),
        json!({"spec": {"template": {"spec": {
            "initContainers": [{"name": "init", "image": "busybox:2"}],
            "containers": [{"name": "db", "image": "pg:16"}],
        }}}})
    );
    let error = plan_image_changes(&slots, &[ImageChange::new("db", "")]).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
}
