//! The Secrets kind (E12-S11): the panel in the detail, masked by default, with the type's facts,
//! reveal and "Edit decoded" going out as commands.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use serde_json::{Value, json};

use crate::detail::tests::fixture::{Detail, kind};
use crate::kinds::{self, KindSpecs};
use crate::table::tests::fixture::cluster;

fn b64(text: &str) -> String {
    STANDARD.encode(text)
}

fn secret(name: &str, kind: &str, data: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": name, "namespace": "shop", "resourceVersion": "5"},
        "type": kind, "data": data
    }))
    .unwrap()
}

fn target(name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Secret"), "shop", name)
}

fn opaque() -> Resource {
    secret(
        "creds",
        "Opaque",
        json!({"username": b64("admin"), "password": b64("super-secret"), "blob": "//4="}),
    )
}

fn register(d: &mut Detail) {
    d.f.vcx.update(|_, cx| kinds::secret::init(cx));
}

#[gpui::test]
fn a_secret_detail_has_its_panel_with_the_keys_masked(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [opaque()]);
    register(&mut d);
    let view = d.open(&target("creds"));
    assert!(d.shown("detail-kind-panel"));
    assert!(d.shown("secret-panel"));
    assert!(d.shown("secret-type"));
    for key in 0..3 {
        let selector: &'static str = Box::leak(format!("secret-key-{key}").into_boxed_str());
        assert!(d.shown(selector), "{selector}");
    }
    assert!(!d.shown("secret-key-3"));
    assert!(
        !d.shown("detail-secret-key-0"),
        "the generic key list gives way to the panel's"
    );
    assert!(!d.shown("secret-reveal-error"));
    let held = d.read(&view, |v| {
        format!("{:?} {:?}", v.model(), v.secret_summary())
    });
    assert!(
        !held.contains("super-secret") && !held.contains("c3VwZXI"),
        "{held}"
    );
    assert!(d.read(&view, |v| !v.reveal_requested()));
}

#[gpui::test]
fn reveal_values_sends_the_command_and_the_values_appear(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [opaque()]);
    register(&mut d);
    let view = d.open(&target("creds"));
    d.f.dispatcher.clear();
    d.click("secret-reveal");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::SecretReveal {
            target: target("creds"),
            key: None
        }]
    );
    d.settle();
    assert!(d.read(&view, |v| v.secret_values_shown()));
    assert!(
        d.read(&view, |v| v
            .revealed_value("password")
            .and_then(|v| v.text().map(str::to_owned)))
            .as_deref()
            == Some("super-secret")
    );
    d.draw();
    assert!(d.shown("secret-value-1"));

    // The same button hides them again.
    d.click("secret-reveal");
    assert!(!d.read(&view, |v| v.secret_values_shown()));
}

#[gpui::test]
fn an_eye_reveals_one_key(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [opaque()]);
    register(&mut d);
    let view = d.open(&target("creds"));
    d.f.dispatcher.clear();
    // Rows are sorted by name: blob, password, username.
    d.click("secret-eye-1");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::SecretReveal {
            target: target("creds"),
            key: Some("password".into())
        }]
    );
    d.settle();
    assert!(d.read(&view, |v| v.revealed_value("password").is_some()));
    assert!(d.read(&view, |v| v.revealed_value("username").is_none()));
}

#[gpui::test]
fn edit_decoded_is_a_command(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [opaque()]);
    register(&mut d);
    d.open(&target("creds"));
    d.f.dispatcher.clear();
    d.click("secret-edit-decoded");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::SecretEditDecoded {
            target: target("creds")
        }]
    );
}

#[gpui::test]
fn a_registry_secret_shows_its_servers_and_usernames(cx: &mut TestAppContext) {
    let config = oxikube_domain::secret::docker::config_json("ghcr.io", "octo", "tok3n", None);
    let mut d = Detail::new(
        cx,
        [secret(
            "pull",
            "kubernetes.io/dockerconfigjson",
            json!({".dockerconfigjson": b64(&config)}),
        )],
    );
    register(&mut d);
    let view = d.open(&target("pull"));
    assert!(d.shown("secret-fact-registry-0"));
    let held = d.read(&view, |v| format!("{:?}", v.secret_summary()));
    assert!(held.contains("octo") && held.contains("ghcr.io"), "{held}");
    assert!(!held.contains("tok3n"), "never the password: {held}");
}

#[gpui::test]
fn a_tls_secret_shows_its_certificate_and_expiry(cx: &mut TestAppContext) {
    let (cert, key) = tls_pair();
    let mut d = Detail::new(
        cx,
        [secret(
            "web-tls",
            "kubernetes.io/tls",
            json!({"tls.crt": b64(&cert), "tls.key": b64(&key)}),
        )],
    );
    register(&mut d);
    let view = d.open(&target("web-tls"));
    for fact in [
        "secret-fact-tls-0",
        "secret-fact-tls-1",
        "secret-fact-tls-2",
        "secret-fact-tls-3",
    ] {
        assert!(d.shown(fact), "{fact}");
    }
    let held = d.read(&view, |v| format!("{:?}", v.secret_summary()));
    assert!(held.contains("web.example.com"), "{held}");
    assert!(!held.contains("PRIVATE KEY"), "never the key: {held}");
}

/// A self-signed certificate and its key, made at run time (no key material in the repository).
fn tls_pair() -> (String, String) {
    let key = rcgen::KeyPair::generate().unwrap();
    let params = rcgen::CertificateParams::new(vec!["web.example.com".to_owned()]).unwrap();
    let cert = params.self_signed(&key).unwrap();
    (cert.pem(), key.serialize_pem())
}

#[gpui::test]
fn the_spec_lists_the_secret_actions_and_ranks_after_configmaps(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, []);
    d.f.vcx.update(|_, cx| {
        kinds::init(cx);
        let meta = KindSpecs::kind_for(cx, &Gvk::new("", "v1", "Secret"));
        for action in [
            CommandId::SECRET_REVEAL,
            CommandId::SECRET_EDIT_DECODED,
            CommandId::SECRET_CREATE,
        ] {
            assert!(meta.offers(action), "{action}");
        }
        let slot = |kind: &str| {
            KindSpecs::kind_for(cx, &Gvk::new("", "v1", kind))
                .sidebar
                .map(|s| (s.section.id, s.rank))
        };
        assert_eq!(slot("ConfigMap"), Some(("config", 10)));
        assert_eq!(slot("Secret"), Some(("config", 20)));
        assert_eq!(slot("ResourceQuota"), Some(("config", 30)));
    });
    let _ = kind("", "v1", "Secret", "secrets", true);
}

#[gpui::test]
fn the_secret_row_actions_are_offered_for_secrets_only(cx: &mut TestAppContext) {
    let _ = cx;
    let actions = kinds::secret_row_actions();
    assert_eq!(actions.len(), 2);
    let reveal = actions
        .iter()
        .find(|a| a.command == CommandId::SECRET_REVEAL)
        .unwrap();
    let command = (reveal.build)(&target("creds"));
    assert_eq!(
        command,
        Command::SecretReveal {
            target: target("creds"),
            key: None
        }
    );
    assert!(!reveal.bulk, "one object at a time");
    assert!(kinds::is_secret_kind(&Gvk::new("", "v1", "Secret")));
    assert!(!kinds::is_secret_kind(&Gvk::new("", "v1", "ConfigMap")));
}
