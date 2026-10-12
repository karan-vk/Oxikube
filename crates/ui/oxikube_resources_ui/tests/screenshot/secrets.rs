//! The ConfigMap and Secret details (E12-S11): a TLS Secret masked with its certificate, a
//! registry Secret masked, an Opaque Secret revealed (light), and a ConfigMap with an expanded
//! YAML key.
//!
//! The certificate is made at run time from a fixed seed with a fixed validity (2026-01-01 for 90 days), and the
//! detail reads its clock as 2026-01-10, so the expiry line does not change with the day.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oxikube_domain::Resource;
use oxikube_resources_ui::detail::DetailTab;
use oxikube_resources_ui::kinds::ConfigMapPanel;
use serde_json::json;

use super::detail::render_with;
use super::{check, detail};

const WIDTH: f32 = detail::WIDTH;
const HEIGHT: f32 = detail::HEIGHT;

fn b64(text: &str) -> String {
    STANDARD.encode(text)
}

fn secret(name: &str, kind: &str, data: serde_json::Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {
            "name": name, "namespace": "shop", "resourceVersion": "9",
            "creationTimestamp": "2026-01-01T00:00:00Z",
            "labels": {"app": "web"}
        },
        "type": kind, "data": data
    }))
    .expect("a secret")
}

/// A certificate that is the same bytes every run (a fixed Ed25519 seed and serial), so the sizes
/// the panel shows do not change from run to run.
fn tls() -> Resource {
    // PKCS#8 v1 for Ed25519: the fixed prefix, then a 32-byte seed.
    let mut der = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    der.extend([7u8; 32]);
    let key = rcgen::KeyPair::try_from(der.as_slice()).expect("an Ed25519 key");
    let mut params = rcgen::CertificateParams::new(vec![
        "web.example.com".to_owned(),
        "api.example.com".to_owned(),
    ])
    .expect("params");
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "web.example.com");
    params.serial_number = Some(rcgen::SerialNumber::from(1u64));
    let start = rcgen::date_time_ymd(2026, 1, 1);
    params.not_before = start;
    params.not_after = start + std::time::Duration::from_secs(90 * 86_400);
    let cert = params.self_signed(&key).expect("self-signed");
    secret(
        "web-tls",
        "kubernetes.io/tls",
        json!({"tls.crt": b64(&cert.pem()), "tls.key": b64(&key.serialize_pem())}),
    )
}

fn registry() -> Resource {
    let config =
        oxikube_domain::secret::docker::config_json("ghcr.io", "octo", "tok3n-value", None);
    secret(
        "pull",
        "kubernetes.io/dockerconfigjson",
        json!({".dockerconfigjson": b64(&config)}),
    )
}

fn opaque() -> Resource {
    secret(
        "creds",
        "Opaque",
        json!({
            "DB_HOST": b64("db.internal"),
            "DB_PASSWORD": b64("correct horse battery staple"),
            "ca.pem": b64("-----BEGIN CERTIFICATE-----\nMIIB...\n-----END CERTIFICATE-----\n"),
            "blob": "//4="
        }),
    )
}

fn config_map() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {
            "name": "app", "namespace": "shop", "resourceVersion": "4",
            "creationTimestamp": "2026-01-01T00:00:00Z"
        },
        "data": {
            "app.yaml": "server:\n  port: 8080\n  tls: true\nfeatures:\n  - search\n  - export\n",
            "db.properties": "host=db.internal\nport=5432\n",
            "feature-flags.json": "{\"search\": true, \"export\": false}",
            "motd": "welcome"
        },
        "binaryData": {"logo.png": "iVBORw0KGgoAAAANSUhEUg=="}
    }))
    .expect("a config map")
}

pub(crate) fn run() -> anyhow::Result<()> {
    check(
        "detail_secret_tls_dark",
        render_with(tls(), false, DetailTab::Overview, true, |_, _| {})?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_secret_registry_dark",
        render_with(registry(), false, DetailTab::Overview, true, |_, _| {})?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_secret_revealed_light",
        render_with(opaque(), true, DetailTab::Overview, true, |cx, detail| {
            cx.update(|cx| detail.update(cx, |detail, cx| detail.toggle_reveal(None, cx)));
        })?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_secret_yaml_decoded_dark",
        render_with(opaque(), false, DetailTab::Yaml, true, |cx, detail| {
            cx.update(|cx| detail.update(cx, |detail, cx| detail.toggle_reveal(None, cx)));
        })?,
        WIDTH,
        HEIGHT,
    )?;
    check(
        "detail_configmap_preview_dark",
        render_with(
            config_map(),
            false,
            DetailTab::Overview,
            true,
            |cx, detail| {
                let panel = cx.update(|cx| detail.read(cx).kind_panel().cloned());
                let panel = panel
                    .and_then(|panel| panel.downcast::<ConfigMapPanel>().ok())
                    .expect("the ConfigMap panel");
                // The keys are made off the UI thread: let them land, then expand `app.yaml`.
                cx.run_until_parked();
                cx.update(|cx| panel.update(cx, |panel, cx| panel.toggle("app.yaml", cx)));
            },
        )?,
        WIDTH,
        HEIGHT,
    )
}
