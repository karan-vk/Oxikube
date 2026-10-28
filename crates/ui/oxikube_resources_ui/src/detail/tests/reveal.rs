//! Revealing a Secret (E12-S11): `secret::Reveal` reads the Secret once more and holds it decoded
//! in the detail's memory. Over the fakes: the real session, store and `ResourceViews`.

use std::path::PathBuf;
use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::ResourceCall;
use serde_json::json;

use super::fixture::Detail;
use crate::detail::{DetailTab, REVEAL_TIMEOUT, RevealLoad};
use crate::table::tests::fixture::cluster;
use crate::views::ViewRequest;

/// The plaintext of `password` and `username`, and their base64.
const PLAIN: [&str; 2] = ["super-secret", "admin"];
const ENCODED: [&str; 2] = ["c3VwZXItc2VjcmV0", "YWRtaW4="];

fn secret(version: &str) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "creds", "namespace": "shop", "resourceVersion": version},
        "type": "Opaque",
        "data": {"username": ENCODED[1], "password": ENCODED[0], "blob": "//4="}
    }))
    .unwrap()
}

fn secret_ref() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Secret"), "shop", "creds")
}

fn gets(d: &Detail) -> usize {
    d.f.ports()
        .resources
        .recorded_calls()
        .into_iter()
        .filter(|call| matches!(call, ResourceCall::Get { .. }))
        .count()
}

/// Everything the view holds, as text: it must not contain a value until the user asks.
fn held(d: &mut Detail, view: &gpui::Entity<crate::detail::DetailView>) -> String {
    d.read(view, |v| {
        format!(
            "{:?} {:?} {:?} {:?}",
            v.model(),
            v.rows(),
            v.full_state_text(),
            v.secret_summary()
        )
    })
}

#[gpui::test]
fn a_secret_holds_no_value_until_it_is_revealed(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    let held = held(&mut d, &view);
    for leaked in PLAIN.iter().chain(&ENCODED) {
        assert!(!held.contains(leaked), "{leaked} is held: {held}");
    }
    let summary = d
        .read(&view, |v| v.secret_summary().cloned())
        .expect("a summary");
    let keys: Vec<&str> = summary.keys.iter().map(|k| k.key.as_str()).collect();
    assert_eq!(keys, ["blob", "password", "username"]);
    assert!(!d.read(&view, |v| v.reveal_requested()));
    assert!(d.read(&view, |v| v.revealed_value("password").is_none()));
    assert_eq!(gets(&d), 1, "only the full read so far");
}

#[gpui::test]
fn revealing_reads_the_secret_once_more_and_decodes_it_in_memory(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));

    assert_eq!(gets(&d), 2, "one more get for the values");
    assert!(d.read(&view, |v| v.reveals_all() && v.secret_values_shown()));
    let (password, blob) = d.read(&view, |v| {
        (
            v.revealed_value("password")
                .and_then(|v| v.text().map(str::to_owned)),
            v.revealed_value("blob").and_then(|v| v.placeholder()),
        )
    });
    assert_eq!(password.as_deref(), Some("super-secret"));
    assert_eq!(blob.as_deref(), Some("(binary, 2 bytes)"));
    // The row's line is made once, at the read: a redraw takes it, not the value.
    let (line, blob_line) = d.read(&view, |v| {
        (
            v.revealed_line("password").map(ToString::to_string),
            v.revealed_line("blob").map(ToString::to_string),
        )
    });
    assert_eq!(line.as_deref(), Some("super-secret"));
    assert_eq!(blob_line.as_deref(), Some("(binary, 2 bytes)"));
    // The full read and the model never held a value: the plaintext is only in the reveal.
    let model = d.read(&view, |v| {
        format!("{:?} {:?}", v.model(), v.full_state_text())
    });
    assert!(
        !model.contains("super-secret") && !model.contains(ENCODED[0]),
        "{model}"
    );
}

#[gpui::test]
fn x_hides_the_values_again_and_forgets_them(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    assert!(d.read(&view, |v| v.secret_values_shown()));

    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    assert!(!d.read(&view, |v| v.reveal_requested() || v.secret_values_shown()));
    assert!(d.read(&view, |v| v.revealed_value("password").is_none()));
    assert!(
        d.read(&view, |v| v.revealed_line("password").is_none()),
        "no line of a value outlives it"
    );
    assert_eq!(gets(&d), 2, "hiding reads nothing");
}

#[gpui::test]
fn one_key_can_be_revealed_alone(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.update(&view, |v, cx| v.toggle_reveal(Some("password"), cx));
    assert!(d.read(&view, |v| v.revealed_value("password").is_some()));
    assert!(
        d.read(&view, |v| v.revealed_value("username").is_none()),
        "the other keys stay masked"
    );
    assert!(!d.read(&view, |v| v.reveals_all()));

    // A second key, then the first one hidden again.
    d.update(&view, |v, cx| v.toggle_reveal(Some("username"), cx));
    d.update(&view, |v, cx| v.toggle_reveal(Some("password"), cx));
    assert!(d.read(&view, |v| v.revealed_value("password").is_none()));
    assert!(d.read(&view, |v| v.revealed_value("username").is_some()));
    assert_eq!(gets(&d), 2, "the Secret was read once for all of it");

    // The last revealed key hidden: nothing is held.
    d.update(&view, |v, cx| v.toggle_reveal(Some("username"), cx));
    assert!(!d.read(&view, |v| v.secret_values_shown()));
}

#[gpui::test]
fn after_a_full_reveal_one_key_can_be_hidden(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    d.update(&view, |v, cx| v.toggle_reveal(Some("password"), cx));
    assert!(d.read(&view, |v| v.revealed_value("password").is_none()));
    assert!(d.read(&view, |v| v.revealed_value("username").is_some()));
    assert!(!d.read(&view, |v| v.reveals_all()));
}

#[gpui::test]
fn the_reveal_goes_through_the_command_and_the_views(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.f.dispatcher.clear();
    d.update(&view, |v, cx| v.request_reveal(Some("username"), cx));
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::SecretReveal {
            target: secret_ref(),
            key: Some("username".into())
        }]
    );
    // The window's `ResourceViews` applied what the sink queued: the key is on screen.
    d.settle();
    assert!(d.read(&view, |v| v.revealed_value("username").is_some()));

    // `x` with something revealed hides it; the handler opens the detail first when it is closed.
    let views = d.f.views.clone();
    d.f.vcx.update(|window, cx| {
        views.update(cx, |views, cx| {
            views.apply_in(
                ViewRequest::RevealSecret {
                    target: secret_ref(),
                    key: None,
                },
                window,
                cx,
            )
        })
    });
    d.settle();
    assert!(!d.read(&view, |v| v.reveal_requested()), "hidden again");
    d.f.vcx.update(|window, cx| {
        views.update(cx, |views, cx| {
            views.apply_in(
                ViewRequest::RevealSecret {
                    target: secret_ref(),
                    key: None,
                },
                window,
                cx,
            )
        })
    });
    d.settle();
    assert!(d.read(&view, |v| v.reveals_all()));
}

#[gpui::test]
fn the_yaml_tab_shows_the_revealed_values_and_masks_them_again(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.click("detail-tab-yaml");
    let yaml = |d: &mut Detail| {
        d.read(&view, |v| v.yaml().map(str::to_owned))
            .expect("YAML")
    };
    let masked = yaml(&mut d);
    assert!(masked.contains("password: (hidden)"), "{masked}");
    for leaked in PLAIN.iter().chain(&ENCODED) {
        assert!(!masked.contains(leaked), "{masked}");
    }

    d.f.dispatcher.clear();
    d.click("yaml-decode");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::SecretReveal {
            target: secret_ref(),
            key: None
        }],
        "the toggle sends the command"
    );
    d.settle();
    let decoded = yaml(&mut d);
    assert!(decoded.contains("password: super-secret"), "{decoded}");
    assert!(decoded.contains("username: admin"), "{decoded}");
    assert!(decoded.contains("blob: (binary, 2 bytes)"), "{decoded}");
    assert!(!decoded.contains(ENCODED[0]), "no base64 either: {decoded}");

    d.click("yaml-decode");
    d.settle();
    let again = yaml(&mut d);
    assert_eq!(again, masked, "masked again");
}

#[gpui::test]
fn decoded_yaml_is_never_saved_to_a_file(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.click("detail-tab-yaml");
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    assert!(d.read(&view, |v| {
        v.yaml().is_some_and(|y| y.contains("super-secret"))
    }));

    // The toolbar's button is off, and the command is refused with a toast: the file dialog
    // never opens (a pending prompt would be answered below; there is none).
    let views = d.f.views.clone();
    d.f.vcx.update(|window, cx| {
        views.update(cx, |views, cx| {
            views.apply_in(ViewRequest::SaveYaml(secret_ref()), window, cx)
        })
    });
    d.settle();
    d.settle();
    let workspace = d.workspace();
    let toasts = d.f.vcx.update(|_, cx| {
        let layer = workspace.read(cx).toast_layer().clone();
        layer.read(cx).visible()
    });
    assert!(
        toasts
            .iter()
            .any(|t| t.message.contains("never written to disk")),
        "{toasts:?}"
    );
    assert!(
        d.f.fs.file("/home/me/creds.yaml").is_none(),
        "decoded values are never written to disk"
    );

    // Hidden again, saving works as before.
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    d.f.vcx.update(|window, cx| {
        views.update(cx, |views, cx| {
            views.apply_in(ViewRequest::SaveYaml(secret_ref()), window, cx)
        })
    });
    d.f.vcx
        .simulate_new_path_selection(|_| Some(PathBuf::from("/home/me/creds.yaml")));
    d.settle();
    d.settle();
    let saved = d.f.fs.file("/home/me/creds.yaml").expect("saved masked");
    let saved = String::from_utf8(saved).unwrap();
    assert!(saved.contains("password: (hidden)") && !saved.contains("super-secret"));
}

#[gpui::test]
fn the_values_go_after_the_timeout(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    d.f.vcx
        .executor()
        .advance_clock(REVEAL_TIMEOUT - Duration::from_secs(1));
    d.settle();
    assert!(d.read(&view, |v| v.secret_values_shown()), "not yet");

    d.f.vcx.executor().advance_clock(Duration::from_secs(2));
    d.settle();
    assert!(
        !d.read(&view, |v| v.secret_values_shown() || v.reveal_requested()),
        "hidden after {REVEAL_TIMEOUT:?}"
    );
}

#[gpui::test]
fn a_failed_reveal_says_why_and_shows_nothing(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.f.ports()
        .resources
        .script()
        .get
        .push_err(oxikube_domain::OxiError::forbidden(
            "secrets \"creds\" is forbidden",
        ));
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    assert!(!d.read(&view, |v| v.secret_values_shown() || v.reveal_requested()));
    let load = d.read(&view, |v| v.reveal_load().clone());
    assert!(matches!(load, RevealLoad::Failed(message) if message.contains("forbidden")));

    // Asking again after the failure tries again.
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    assert!(d.read(&view, |v| v.secret_values_shown()));
}

#[gpui::test]
fn closing_the_tab_clears_the_values(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    assert!(d.read(&view, |v| v.secret_values_shown()));
    d.update(&view, |v, _| v.release());
    assert!(!d.read(&view, |v| v.secret_values_shown() || v.reveal_requested()));
    assert!(d.read(&view, |v| v.secret_summary().is_none()));
}

#[gpui::test]
fn the_yaml_toggle_is_only_for_secrets(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [super::fixture::web_pod()]);
    let view = d.open(&super::fixture::pod_ref("web-0"));
    d.click("detail-tab-yaml");
    assert!(!d.shown("yaml-decode"));
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    assert!(
        !d.read(&view, |v| v.reveal_requested()),
        "a pod reveals nothing"
    );
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml);
}

#[gpui::test]
fn a_revealed_tab_is_never_saved_with_the_layout(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret("5")]);
    let view = d.open(&secret_ref());
    d.update(&view, |v, cx| v.toggle_reveal(None, cx));
    d.click("detail-tab-yaml");
    assert!(d.read(&view, |v| {
        v.yaml().is_some_and(|y| y.contains("super-secret"))
    }));
    // Pinned as a tab of the cluster's workspace, with the values on screen.
    d.click("detail-pin");
    let workspace = d.workspace();
    let saved = d.f.vcx.update(|_, cx| {
        serde_json::to_string(&workspace.read(cx).serialize_layout(cx)).expect("the layout")
    });
    for leaked in PLAIN.iter().chain(&ENCODED) {
        assert!(
            !saved.contains(leaked),
            "{leaked} reached the layout: {saved}"
        );
    }
    assert!(
        <crate::detail::DetailView as oxikube_workspace::Item>::serialized_kind().is_none(),
        "a detail tab is not restored, so nothing of it is stored"
    );
}
