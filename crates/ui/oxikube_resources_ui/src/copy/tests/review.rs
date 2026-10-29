//! The review: an object that exists (overwrite, skip, rename), force over other managers' fields,
//! a missing namespace, references the target lacks.

use gpui::TestAppContext;

use super::*;
use crate::copy::{Conflict, CopyStage};

fn existing(rig: &Rig) {
    rig.cluster("b")
        .insert(object(json!({"apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "prod"}, "data": {"mode": "green"}})));
}

/// The review of `source` into `prod` of `b`.
fn review(rig: &mut Rig, source: ResourceRef) -> Entity<CopyDialog> {
    rig.begin(source);
    rig.vcx.simulate_keystrokes("down enter");
    rig.settle();
    rig.vcx.simulate_input("prod");
    rig.settle();
    rig.vcx.simulate_keystrokes("enter");
    rig.settle();
    rig.dialog().expect("the review is open")
}

fn can_copy(rig: &mut Rig, dialog: &Entity<CopyDialog>) -> bool {
    rig.vcx.update(|_, cx| dialog.read(cx).can_copy(cx))
}

#[gpui::test]
fn an_existing_object_needs_a_choice_and_skip_writes_nothing(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    existing(&rig);
    let source = rig.source(configmap());
    let dialog = review(&mut rig, source);
    assert!(
        !can_copy(&mut rig, &dialog),
        "overwrite, skip or rename first"
    );
    rig.vcx.update(|window, cx| window.draw(cx).clear(cx));
    for selector in [
        "copy-conflict",
        "copy-exists",
        "copy-overwrite",
        "copy-skip",
        "copy-rename",
    ] {
        assert!(
            rig.vcx.debug_bounds(selector).is_some(),
            "{selector} is drawn"
        );
    }
    rig.vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.choose(Conflict::Skip, window, cx)));
    assert!(can_copy(&mut rig, &dialog));
    dialog.update(&mut rig.vcx, |dialog, cx| dialog.submit(cx));
    rig.settle();
    assert_eq!(rig.real_writes("b"), 0);
    assert!(rig.dialog().is_none());
    assert!(
        rig.toasts()
            .iter()
            .any(|(_, text)| text.contains("Skipped"))
    );
}

#[gpui::test]
fn overwrite_replaces_the_object_at_the_target(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    existing(&rig);
    let source = rig.source(configmap());
    let dialog = review(&mut rig, source);
    rig.vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.choose(Conflict::Overwrite, window, cx)));
    dialog.update(&mut rig.vcx, |dialog, cx| dialog.submit(cx));
    rig.settle();
    assert_eq!(rig.real_writes("b"), 1);
    let stored = rig
        .cluster("b")
        .objects()
        .into_iter()
        .find(|o| o.name() == "settings")
        .unwrap();
    assert_eq!(stored.to_value()["data"]["mode"], "blue");
}

#[gpui::test]
fn rename_suggests_a_copy_name_and_reviews_the_copy_under_it(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    existing(&rig);
    let source = rig.source(configmap());
    let dialog = review(&mut rig, source);
    rig.vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.choose(Conflict::Rename, window, cx)));
    rig.settle();
    let (name, exists, stage) = rig.vcx.update(|_, cx| {
        let d = dialog.read(cx);
        (d.name_text(cx), d.preview().map(|p| p.exists()), d.stage())
    });
    assert_eq!(name, "settings-copy");
    assert_eq!(exists, Some(false));
    assert_eq!(stage, CopyStage::Ready);
    dialog.update(&mut rig.vcx, |dialog, cx| dialog.submit(cx));
    rig.settle();
    let names: Vec<String> = rig
        .cluster("b")
        .objects()
        .iter()
        .map(|o| o.name().to_owned())
        .collect();
    assert!(names.contains(&"settings-copy".to_owned()), "{names:?}");
    let original = rig
        .cluster("b")
        .objects()
        .into_iter()
        .find(|o| o.name() == "settings")
        .unwrap();
    assert_eq!(
        original.to_value()["data"]["mode"],
        "green",
        "the original is untouched"
    );
}

#[gpui::test]
fn a_namespace_that_does_not_exist_is_created_as_its_own_step(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let source = rig.source(configmap());
    rig.begin(source);
    rig.vcx.simulate_keystrokes("down enter");
    rig.settle();
    rig.vcx.simulate_input("brand-new");
    rig.settle();
    rig.vcx.simulate_keystrokes("enter");
    rig.settle();
    let dialog = rig.dialog().expect("the review is open");
    let error = rig.vcx.update(|_, cx| dialog.read(cx).error().cloned());
    assert_eq!(
        error,
        Some(oxikube_app::CopyError::NamespaceMissing("brand-new".into()))
    );
    assert!(!can_copy(&mut rig, &dialog));
    rig.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(rig.vcx.debug_bounds("copy-create-namespace").is_some());
    assert_eq!(rig.real_writes("b"), 0, "nothing was created silently");

    dialog.update(&mut rig.vcx, |dialog, cx| dialog.create_namespace(cx));
    rig.settle();
    assert_eq!(rig.real_writes("b"), 1, "the namespace");
    assert!(can_copy(&mut rig, &dialog), "the review ran again");
    dialog.update(&mut rig.vcx, |dialog, cx| dialog.submit(cx));
    rig.settle();
    assert_eq!(rig.real_writes("b"), 2);
}

#[gpui::test]
fn missing_references_are_listed_as_warnings(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let source = rig.source(json!({"apiVersion": "v1", "kind": "Pod",
        "metadata": {"name": "p", "namespace": "shop"},
        "spec": {"serviceAccountName": "robot", "containers": [{"name": "c", "image": "x",
            "envFrom": [{"configMapRef": {"name": "settings"}}]}]}}));
    let dialog = review(&mut rig, source);
    let missing = rig.vcx.update(|_, cx| {
        dialog
            .read(cx)
            .preview()
            .map(|p| p.missing.iter().map(|m| m.message()).collect::<Vec<_>>())
    });
    let missing = missing.expect("a review");
    assert_eq!(missing.len(), 2, "{missing:?}");
    rig.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(rig.vcx.debug_bounds("copy-missing").is_some());
    // Only warnings: the copy still goes ahead.
    assert!(can_copy(&mut rig, &dialog));
}

#[gpui::test]
fn editing_the_name_without_enter_disables_the_copy_until_it_is_reviewed(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    let source = rig.source(configmap());
    let dialog = review(&mut rig, source);
    assert!(can_copy(&mut rig, &dialog));
    rig.vcx
        .update(|window, cx| dialog.update(cx, |d, cx| d.set_name("settings-2", window, cx)));
    rig.settle();
    assert!(
        !can_copy(&mut rig, &dialog),
        "the review was made for the old name"
    );
    dialog.update(&mut rig.vcx, |dialog, cx| dialog.review(cx));
    rig.settle();
    assert!(can_copy(&mut rig, &dialog));
    dialog.update(&mut rig.vcx, |dialog, cx| dialog.submit(cx));
    rig.settle();
    assert!(
        rig.cluster("b")
            .objects()
            .iter()
            .any(|o| o.name() == "settings-2")
    );
}
