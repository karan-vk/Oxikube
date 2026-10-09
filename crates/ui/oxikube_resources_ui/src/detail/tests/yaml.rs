//! The YAML tab: the object as read-only highlighted YAML, managedFields, Secret masking, copy
//! and save. Over the fakes: a real session manager and store, the real `ResourceViews`
//! applying the commands the buttons send.

use std::path::PathBuf;
use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::Timeline;
use serde_json::json;

use super::fixture::{Detail, edited, pod_ref, web_pod};
use crate::detail::{DetailTab, DetailView};
use crate::table::tests::fixture::cluster;

const SECRET_VALUES: [&str; 4] = [
    "YWRtaW4=",
    "c3VwZXItc2VjcmV0",
    "plain-token-value",
    "super-secret",
];

fn secret() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {
            "name": "creds", "namespace": "shop", "resourceVersion": "5",
            "creationTimestamp": "2026-01-01T00:00:00Z",
            "annotations": {
                "kubectl.kubernetes.io/last-applied-configuration":
                    "{\"data\":{\"password\":\"c3VwZXItc2VjcmV0\"}}",
                "team": "blue"
            }
        },
        "type": "Opaque",
        "data": {"username": "YWRtaW4=", "password": "c3VwZXItc2VjcmV0"},
        "stringData": {"token": "plain-token-value"}
    }))
    .unwrap()
}

fn secret_ref() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Secret"), "shop", "creds")
}

/// A pod with `managedFields`.
fn managed_pod() -> Resource {
    edited(web_pod(), |json| {
        json["metadata"]["managedFields"] = json!([{
            "manager": "kubectl-client-side-apply", "operation": "Update",
            "apiVersion": "v1", "fieldsType": "FieldsV1",
            "fieldsV1": {"f:metadata": {"f:labels": {".": {}, "f:app": {}}}}
        }]);
    })
}

/// The text the YAML tab's code view has on screen.
fn on_screen(d: &mut Detail, view: &gpui::Entity<DetailView>) -> Option<String> {
    let code = d.read(view, |v| v.yaml.view.clone())?;
    d.f.vcx
        .update(|_, cx| code.read(cx).text().map(|text| text.to_string()))
}

fn clipboard(d: &mut Detail) -> Option<String> {
    d.f.vcx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui::test]
fn the_yaml_tab_shows_the_object_in_a_read_only_highlighted_view(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [managed_pod()]);
    let view = d.open(&pod_ref("web-0"));
    assert_eq!(
        d.read(&view, |v| v.yaml().map(str::to_owned)),
        None,
        "not made until shown"
    );

    d.click("detail-tab-yaml");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml);
    let yaml = d
        .read(&view, |v| v.yaml().map(str::to_owned))
        .expect("the YAML");
    assert!(yaml.contains("kind: Pod"), "{yaml}");
    assert!(yaml.contains("name: web-0"), "{yaml}");
    assert!(d.shown("detail-yaml-editor"), "the code view is on screen");
    assert!(
        !d.shown("detail-yaml-loading"),
        "no skeleton once it is laid out"
    );

    // The code view is read-only by construction (E10 is the editor).
    let code = d.read(&view, |v| v.yaml.view.clone()).expect("a code view");
    let (language, coloured) = d.f.vcx.update(|_, cx| {
        let code = code.read(cx);
        (code.language(), code.is_highlighted())
    });
    assert_eq!(language, Some("yaml"), "tree-sitter YAML highlighting");
    assert!(coloured, "parsed off the UI thread");
    assert_eq!(
        on_screen(&mut d, &view).as_deref(),
        Some(yaml.as_str()),
        "the view holds exactly the text"
    );
}

#[gpui::test]
fn managed_fields_are_hidden_until_toggled_and_the_cached_object_is_untouched(
    cx: &mut TestAppContext,
) {
    let mut d = Detail::new(cx, [managed_pod()]);
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-yaml");
    let yaml = |d: &mut Detail, view: &gpui::Entity<DetailView>| {
        d.read(view, |v| v.yaml().map(str::to_owned)).expect("YAML")
    };
    assert!(
        !yaml(&mut d, &view).contains("managedFields"),
        "hidden by default"
    );
    assert!(!d.read(&view, |v| v.managed_fields_shown()));

    d.f.dispatcher.clear();
    d.click("yaml-managed");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceToggleManagedFields {
            target: pod_ref("web-0")
        }],
        "the button sends the command"
    );
    let shown = yaml(&mut d, &view);
    assert!(shown.contains("managedFields"), "{shown}");
    assert!(shown.contains("kubectl-client-side-apply"), "{shown}");
    d.draw();
    assert_eq!(
        on_screen(&mut d, &view),
        Some(shown),
        "the view follows the toggle"
    );

    // The object the store (and the Overview) hold never lost its managedFields.
    let kept = d.read(&view, |v| {
        v.complete_resource()
            .map(|r| r.to_value()["metadata"].get("managedFields").is_some())
    });
    assert_eq!(kept, Some(true));

    d.click("yaml-managed");
    assert!(
        !yaml(&mut d, &view).contains("managedFields"),
        "hidden again"
    );
}

#[gpui::test]
fn the_yaml_follows_the_object_when_it_changes(cx: &mut TestAppContext) {
    let pod = web_pod();
    let mut d = Detail::new(cx, [pod.clone()]);
    let updated = edited(pod.clone(), |json| {
        json["metadata"]["resourceVersion"] = json!("8");
        json["metadata"]["labels"]["app"] = json!("web-v2");
    });
    // The feed: the pod, then a new version a second later.
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![pod])])])
            .ok_at(
                Duration::from_secs(1),
                DeltaBatch::from_deltas(vec![Delta::Applied(updated)]),
            )
            .keep_open(),
    );
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-yaml");
    assert!(d.read(&view, |v| {
        v.yaml().is_some_and(|y| y.contains("app: web\n"))
    }));

    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(1));
    d.settle();
    d.draw();
    let yaml = d.read(&view, |v| v.yaml().map(str::to_owned)).unwrap();
    assert!(yaml.contains("app: web-v2"), "{yaml}");
    assert_eq!(
        on_screen(&mut d, &view),
        Some(yaml),
        "the view was given the new text"
    );
}

#[gpui::test]
fn a_secrets_yaml_is_masked_in_the_view_the_clipboard_and_the_file(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret()]);
    let view = d.open(&secret_ref());
    d.click("detail-tab-yaml");
    d.settle();
    let yaml = d
        .read(&view, |v| v.yaml().map(str::to_owned))
        .expect("the YAML");
    for value in SECRET_VALUES {
        assert!(!yaml.contains(value), "{value} is shown:\n{yaml}");
    }
    for key in [
        "username: (hidden)",
        "password: (hidden)",
        "token: (hidden)",
    ] {
        assert!(yaml.contains(key), "{key} missing:\n{yaml}");
    }
    assert!(!yaml.contains("last-applied-configuration"), "{yaml}");
    assert!(yaml.contains("team: blue"), "{yaml}");
    assert_eq!(on_screen(&mut d, &view).as_deref(), Some(yaml.as_str()));

    // Copy: the masked text, on the (fake) clipboard.
    d.f.dispatcher.clear();
    d.click("yaml-copy");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceCopyYaml {
            target: secret_ref()
        }]
    );
    assert_eq!(clipboard(&mut d).as_deref(), Some(yaml.as_str()));

    // Save: a file dialog, then exactly the displayed text through the FsPort.
    d.f.dispatcher.clear();
    d.click("yaml-save");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceSaveYaml {
            target: secret_ref()
        }]
    );
    let mut suggested = None;
    d.f.vcx.simulate_new_path_selection(|dir| {
        suggested = Some(dir.to_path_buf());
        Some(PathBuf::from("/home/me/creds.yaml"))
    });
    d.settle();
    d.settle();
    let written =
        d.f.fs
            .file("/home/me/creds.yaml")
            .expect("the file was written");
    assert_eq!(
        String::from_utf8(written).unwrap(),
        yaml,
        "what is displayed is what is saved"
    );
    for value in SECRET_VALUES {
        assert!(!yaml.contains(value));
    }
    assert!(suggested.is_some(), "the dialog opened somewhere");
    assert_eq!(d.read(&view, |v| v.yaml_file_name()), "creds.yaml");
}

#[gpui::test]
fn cancelling_the_save_dialog_writes_nothing(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.open(&pod_ref("web-0"));
    d.click("detail-tab-yaml");
    d.click("yaml-save");
    d.f.vcx.simulate_new_path_selection(|_| None);
    d.settle();
    d.settle();
    assert!(
        d.f.fs.recorded_calls().is_empty(),
        "no write after a cancelled dialog: {:?}",
        d.f.fs.recorded_calls()
    );
}

#[gpui::test]
fn a_failed_save_is_reported_not_swallowed(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.open(&pod_ref("web-0"));
    d.click("detail-tab-yaml");
    d.f.fs
        .script()
        .write
        .push_err(oxikube_domain::OxiError::forbidden("read-only volume"));
    d.click("yaml-save");
    d.f.vcx
        .simulate_new_path_selection(|_| Some(PathBuf::from("/ro/web-0.yaml")));
    d.settle();
    d.settle();
    assert_eq!(d.f.fs.file("/ro/web-0.yaml"), None);
    let workspace = d.workspace();
    let toasts = d.f.vcx.update(|_, cx| {
        let layer = workspace.read(cx).toast_layer().clone();
        layer.read(cx).visible()
    });
    assert!(
        toasts.iter().any(|t| t.message.contains("Could not save")),
        "{toasts:?}"
    );
}

#[gpui::test]
fn a_metadata_only_object_shows_a_skeleton_until_it_is_read_in_full(cx: &mut TestAppContext) {
    // Secrets and ConfigMaps arrive metadata-only: the YAML waits for the full read.
    let config_map = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "shop", "resourceVersion": "3"},
        "data": {"mode": "fast"}
    }))
    .unwrap();
    let mut d = Detail::new(cx, [config_map]);
    let target = ResourceRef::namespaced(
        cluster(),
        Gvk::new("", "v1", "ConfigMap"),
        "shop",
        "settings",
    );
    let view = d.open(&target);
    d.click("detail-tab-yaml");
    d.settle();
    let yaml = d
        .read(&view, |v| v.yaml().map(str::to_owned))
        .expect("read in full");
    assert!(
        yaml.contains("mode: fast"),
        "the data came with the full read:\n{yaml}"
    );
}

/// A ConfigMap of about 100 KB of YAML.
fn big_config_map() -> Resource {
    let data: serde_json::Map<String, serde_json::Value> = (0..800)
        .map(|i| {
            (
                format!("key-{i:05}"),
                serde_json::Value::String(format!("value {i} {}", "x".repeat(100))),
            )
        })
        .collect();
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "big", "namespace": "shop", "resourceVersion": "1"},
        "data": data,
    }))
    .unwrap()
}

#[gpui::test]
fn a_100_kb_object_is_made_once_per_version_off_the_ui_thread(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [big_config_map()]);
    let target = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "ConfigMap"), "shop", "big");
    let view = d.open(&target);
    d.click("detail-tab-yaml");
    d.settle();

    let first = d
        .read(&view, |v| v.yaml.text.as_ref().map(|t| t.result.clone()))
        .expect("made")
        .expect("ok");
    assert!(first.len() > 100_000, "{} bytes", first.len());
    // The same version again (a redraw, a feed delivering it twice): the text is not made again.
    d.update(&view, |v, cx| v.rebuild(cx));
    d.update(&view, |v, cx| v.rebuild(cx));
    let again = d
        .read(&view, |v| v.yaml.text.as_ref().map(|t| t.result.clone()))
        .unwrap()
        .unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&first, &again),
        "one text per object version"
    );
    // The view holds that same text (no copy), laid out once.
    let code = d.read(&view, |v| v.yaml.view.clone()).expect("a code view");
    let held =
        d.f.vcx
            .update(|_, cx| code.read(cx).text().cloned())
            .expect("laid out");
    assert!(
        std::sync::Arc::ptr_eq(&held, &first),
        "the view shares the text"
    );
    assert_eq!(
        d.read(&view, |v| v.yaml.made),
        1,
        "made once, off the UI thread"
    );
    let started = std::time::Instant::now();
    for _ in 0..10 {
        d.draw();
    }
    let elapsed = started.elapsed();
    assert_eq!(
        d.read(&view, |v| v.yaml.made),
        1,
        "a frame must not make the text again"
    );
    let drawn = d.f.vcx.update(|_, cx| code.read(cx).rows_drawn());
    assert!(drawn < 200, "only the visible rows are built: {drawn}");
    // A re-parse of 100 KB per frame would take far longer than this (debug build, loaded CI).
    assert!(
        elapsed < Duration::from_secs(2),
        "10 frames with a 100 KB YAML tab took {elapsed:?}"
    );
}

#[gpui::test]
fn switching_to_yaml_writes_and_lays_out_nothing_on_the_ui_thread(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [big_config_map()]);
    let target = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "ConfigMap"), "shop", "big");
    let view = d.open(&target);
    d.settle();
    // The switch itself (what `resource_detail::ShowTab` does), without letting tasks run.
    d.f.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.set_tab(DetailTab::Yaml, cx)));
    assert_eq!(
        d.read(&view, |v| v.yaml().map(str::len)),
        None,
        "the YAML is written on the background executor, not in the switch"
    );
    assert!(d.read(&view, |v| v.yaml.making.is_some()));
    // The frame that shows the tab draws the skeleton; it neither writes nor lays out the text.
    assert!(d.shown("detail-yaml-loading"));
    assert_eq!(d.read(&view, |v| v.yaml.made), 0);
    let code = d.read(&view, |v| v.yaml.view.clone()).expect("a code view");
    assert!(!d.f.vcx.update(|_, cx| code.read(cx).is_ready()));

    d.settle();
    d.draw();
    assert!(d.read(&view, |v| v.yaml().is_some_and(|y| y.len() > 100_000)));
    assert!(d.f.vcx.update(|_, cx| code.read(cx).is_current()));
    assert!(
        !d.shown("detail-yaml-loading"),
        "the text replaced the skeleton"
    );
}

#[gpui::test]
fn a_failed_re_read_does_not_leave_the_old_yaml_to_copy_or_save(cx: &mut TestAppContext) {
    // A ConfigMap arrives metadata-only, so its YAML depends on the full read.
    let config_map = Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "shop", "resourceVersion": "3"},
        "data": {"mode": "fast"}
    }))
    .unwrap();
    let mut d = Detail::new(cx, [config_map]);
    let target = ResourceRef::namespaced(
        cluster(),
        Gvk::new("", "v1", "ConfigMap"),
        "shop",
        "settings",
    );
    let view = d.open(&target);
    d.click("detail-tab-yaml");
    d.settle();
    assert!(
        d.read(&view, |v| v.yaml().is_some()),
        "the first read succeeded"
    );

    // The object is read again (a new version) and the read fails.
    d.f.ports()
        .resources
        .script()
        .get
        .push_err(oxikube_domain::OxiError::network("api server unreachable"));
    d.update(&view, |v, cx| v.fetch_full(cx));
    assert_eq!(
        d.read(&view, |v| v.yaml().map(str::to_owned)),
        None,
        "the old text is not offered to copy or save"
    );
    assert!(
        d.shown("detail-yaml-error"),
        "the tab says the object could not be read"
    );
}
