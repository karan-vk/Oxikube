//! Secrets: key names, never values.

use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};
use serde_json::json;

use super::fixture::Detail;
use crate::detail::DetailView;
use crate::detail::model::{Row, Section};
use crate::table::tests::fixture::cluster;

const VALUES: [&str; 3] = ["YWRtaW4=", "c3VwZXItc2VjcmV0", "plain-token-value"];

fn secret() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {
            "name": "creds", "namespace": "shop", "resourceVersion": "5",
            "creationTimestamp": "2026-01-01T00:00:00Z",
            "labels": {"app": "web"},
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

/// Everything the view holds that could be drawn, as text.
fn everything(view: &DetailView) -> String {
    format!(
        "{:?} {:?} {:?}",
        view.model(),
        view.rows(),
        view.full_state_text()
    )
}

#[gpui::test]
fn a_secret_shows_key_names_and_no_value_in_what_is_drawn(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret()]);
    let view = d.open(&secret_ref());
    d.settle();

    let model = d.read(&view, |v| v.model().cloned()).expect("a model");
    assert_eq!(
        model.secret_keys.as_deref(),
        Some(
            &[
                "password".to_owned(),
                "token".to_owned(),
                "username".to_owned()
            ][..]
        ),
        "the key names are read from the object"
    );
    let held = d.read(&view, everything);
    for value in VALUES {
        assert!(
            !held.contains(value),
            "{value} is in what the view holds: {held}"
        );
    }
    assert!(d.shown("detail-secret-key-0"));
    assert!(d.shown("detail-secret-key-2"));
    assert!(!d.shown("detail-secret-key-3"));
    assert!(
        !d.shown("detail-condition-head"),
        "a secret has no conditions table"
    );

    // The annotation that embeds the data is not shown, nor offered for copy.
    let hidden = model
        .meta_entry("kubectl.kubernetes.io/last-applied-configuration", true)
        .expect("listed by name");
    assert_eq!(&*hidden.value, "(hidden)");
    assert_eq!(
        d.read(&view, |v| v.copy_text(
            "kubectl.kubernetes.io/last-applied-configuration",
            true
        )),
        None
    );
    assert_eq!(
        d.read(&view, |v| v.copy_text("team", true)).as_deref(),
        Some("team=blue")
    );
}

#[gpui::test]
fn a_secret_that_arrives_whole_is_still_only_key_names(cx: &mut TestAppContext) {
    // Not what the store does (it watches Secrets metadata-only), but the model must not depend
    // on that: a whole Secret in the feed shows no value either.
    let mut d = Detail::new(cx, [secret()]);
    let view = d.open(&secret_ref());
    d.settle();
    let object = d.f.ports().resources.objects().remove(0);
    let model = crate::detail::DetailModel::build(
        &oxikube_app::store::StoreObject::Resource(object),
        &Gvk::new("", "v1", "Secret"),
        None,
        None,
    );
    let text = format!("{model:?}");
    for value in VALUES {
        assert!(!text.contains(value));
    }
    assert_eq!(model.secret_keys.as_ref().map(Vec::len), Some(3));
    let _ = view;
}

#[gpui::test]
fn the_full_read_masks_values_before_the_view_sees_them(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret()]);
    let view = d.open(&secret_ref());
    d.settle();
    let gets =
        d.f.ports()
            .resources
            .recorded_calls()
            .into_iter()
            .filter(|c| matches!(c, oxikube_testkit::ResourceCall::Get { .. }))
            .count();
    assert_eq!(gets, 1, "the keys need one full read");
    let held = d.read(&view, |v| v.full_state_text());
    assert!(held.contains("password"), "the key names are kept: {held}");
    for value in VALUES {
        assert!(!held.contains(value), "{value} survived the read");
    }
}

#[gpui::test]
fn a_secret_whose_read_is_denied_says_so_instead_of_no_keys(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret()]);
    d.f.ports()
        .resources
        .script()
        .get
        .push_err(oxikube_domain::OxiError::forbidden(
            "secrets \"creds\" is forbidden",
        ));
    let view = d.open(&secret_ref());
    d.settle();
    let rows = d.read(&view, |v| v.rows().to_vec());
    assert!(
        rows.contains(&Row::Loading),
        "the failure row stands where the keys would be: {rows:?}"
    );
    assert!(!rows.contains(&Row::Empty(Section::Keys)));
}

#[gpui::test]
fn a_failed_full_read_is_tried_again_after_a_reconnect(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [secret()]);
    d.f.ports()
        .resources
        .script()
        .get
        .push_err(oxikube_domain::OxiError::network("api server unreachable"));
    let view = d.open(&secret_ref());
    d.settle();
    assert!(
        d.read(&view, |v| v.rows().contains(&Row::Loading)),
        "the first read failed, so the keys are not known"
    );
    let gets = |d: &Detail| {
        d.f.ports()
            .resources
            .recorded_calls()
            .into_iter()
            .filter(|call| matches!(call, oxikube_testkit::ResourceCall::Get { .. }))
            .count()
    };
    assert_eq!(gets(&d), 1);

    // The same version arrives again on the new store: the read is asked for again.
    let sessions = d.f.sessions.clone();
    sessions.disconnect(&cluster()).expect("disconnect");
    d.settle();
    // A real reconnect hands out new ports, so the registry builds a new store; the fake keeps
    // its ports, so the old store is forgotten by hand.
    d.f.deps.stores.remove(&cluster());
    futures::executor::block_on(sessions.connect(&cluster())).expect("connect");
    d.settle();
    assert_eq!(gets(&d), 2, "a reconnect retries the failed read");
    let model = d.read(&view, |v| v.model().cloned()).expect("a model");
    assert!(model.secret_keys.is_some(), "the keys are shown now");
}
