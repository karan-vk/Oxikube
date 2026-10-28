//! The kubeconfig text.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oxikube_domain::secret::SecretText;
use serde_json::Value;

use super::{CA, TOKEN};
use crate::sa_kubeconfig::{KubeconfigSpec, MASK, Tokens};

fn spec() -> KubeconfigSpec {
    KubeconfigSpec {
        cluster_name: "prod".into(),
        server: "https://prod.example:6443".into(),
        certificate_authority_pem: Some(CA.into()),
        insecure_skip_tls_verify: false,
        tls_server_name: None,
        service_account: "deployer".into(),
        namespace: "shop".into(),
        token: SecretText::new(TOKEN),
        notes: vec!["Kubeconfig for deployer.".into(), "Expires at noon.".into()],
    }
}

fn parse(text: &str) -> Value {
    serde_saphyr::from_str(text).expect("the text is YAML")
}

#[test]
fn the_kubeconfig_has_the_server_the_ca_the_token_and_a_named_context() {
    let text = spec().render(Tokens::Revealed).unwrap();
    let config = parse(&text);
    assert_eq!(config["apiVersion"], "v1");
    assert_eq!(config["kind"], "Config");
    let cluster = &config["clusters"][0];
    assert_eq!(cluster["name"], "prod");
    assert_eq!(cluster["cluster"]["server"], "https://prod.example:6443");
    let ca = cluster["cluster"]["certificate-authority-data"]
        .as_str()
        .unwrap();
    assert_eq!(STANDARD.decode(ca).unwrap(), CA.as_bytes());
    assert_eq!(config["users"][0]["name"], "deployer");
    assert_eq!(config["users"][0]["user"]["token"], TOKEN);
    assert_eq!(config["contexts"][0]["name"], "deployer@prod");
    assert_eq!(config["contexts"][0]["context"]["cluster"], "prod");
    assert_eq!(config["contexts"][0]["context"]["user"], "deployer");
    assert_eq!(config["contexts"][0]["context"]["namespace"], "shop");
    assert_eq!(config["current-context"], "deployer@prod");
}

#[test]
fn the_preview_masks_the_token_and_nothing_else() {
    let text = spec().render(Tokens::Masked).unwrap();
    assert!(!text.contains(TOKEN));
    assert!(!text.contains("sa-token-payload"));
    let config = parse(&text);
    assert_eq!(config["users"][0]["user"]["token"], MASK);
    assert_eq!(
        config["clusters"][0]["cluster"]["server"],
        "https://prod.example:6443"
    );
}

#[test]
fn notes_become_comments_above_the_document() {
    let text = spec().render(Tokens::Revealed).unwrap();
    assert!(text.starts_with("# Kubeconfig for deployer.\n# Expires at noon.\n"));
}

#[test]
fn a_cluster_without_a_ca_or_with_skipped_verification_says_so() {
    let mut open = spec();
    open.certificate_authority_pem = None;
    open.insecure_skip_tls_verify = true;
    open.tls_server_name = Some("kubernetes".into());
    let config = parse(&open.render(Tokens::Revealed).unwrap());
    let cluster = &config["clusters"][0]["cluster"];
    assert!(cluster.get("certificate-authority-data").is_none());
    assert_eq!(cluster["insecure-skip-tls-verify"], true);
    assert_eq!(cluster["tls-server-name"], "kubernetes");
}

#[test]
fn debug_prints_no_token() {
    assert!(!format!("{:?}", spec()).contains("sa-token-payload"));
}

#[test]
fn a_long_token_and_ca_stay_on_one_line_each_like_kubectl_writes_them() {
    let mut long = spec();
    long.token = SecretText::new(format!("eyJ{}", "a".repeat(1500)));
    long.certificate_authority_pem = Some(format!(
        "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
        "B".repeat(1800)
    ));
    let text = long.render(Tokens::Revealed).unwrap();
    assert!(
        !text.contains(">-") && !text.contains("|-"),
        "no block scalars:\n{text}"
    );
    let config = parse(&text);
    assert_eq!(config["users"][0]["user"]["token"], long.token.expose());
}
