//! Clusters without `TokenRequest`: a legacy token Secret, with a warning, only when allowed.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::Resource;
use oxikube_domain::audit::Initiator;
use oxikube_testkit::ResourceCall;
use serde_json::json;

use super::{CLUSTER, TOKEN, fixture, generate};
use crate::command_bus::{DispatchError, Outcome};
use crate::sa_kubeconfig::{LONG_LIVED_WARNING, TokenSource};
use crate::testing::ctx;

fn legacy_secret(name: &str, account: &str, token: Option<&str>) -> Resource {
    let mut object = json!({
        "apiVersion": "v1", "kind": "Secret", "type": "kubernetes.io/service-account-token",
        "metadata": {
            "name": name, "namespace": "shop",
            "annotations": {"kubernetes.io/service-account.name": account},
        },
    });
    if let Some(token) = token {
        object["data"] = json!({"token": STANDARD.encode(token)});
    }
    Resource::from_json(object).expect("a Secret")
}

fn without_token_request(f: &super::Fixture) {
    f.h.resources(CLUSTER)
        .script()
        .create_subresource
        .push_err(OxiError::not_found(
            "the server could not find the requested resource",
        ));
}

fn creates(f: &super::Fixture) -> usize {
    f.h.writes(CLUSTER)
        .iter()
        .filter(|call| matches!(call, ResourceCall::Create { .. }))
        .count()
}

#[test]
fn without_consent_the_cluster_is_refused_with_an_explanation() {
    let f = fixture();
    without_token_request(&f);
    let error =
        f.h.confirm_and_run(generate(None, false), ctx(Initiator::Ui));
    let Err(DispatchError::Handler(error)) = error else {
        panic!("{error:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(
        error.message().contains("no TokenRequest"),
        "{}",
        error.message()
    );
    assert_eq!(creates(&f), 0, "no Secret is made without consent");
    assert!(f.vault.is_empty());
}

#[test]
fn an_existing_token_secret_is_used_and_the_file_carries_the_warning() {
    let f = fixture();
    without_token_request(&f);
    // The fake does not evaluate field selectors, so it answers the list the API server would.
    f.h.resources(CLUSTER)
        .script()
        .list
        .push_ok(oxikube_ports::ListPage::complete(vec![
            legacy_secret("other-token", "builder", Some("not-ours")),
            legacy_secret("deployer-token-x", "deployer", Some(TOKEN)),
        ]));

    let outcome =
        f.h.confirm_and_run(generate(None, true), ctx(Initiator::Ui))
            .unwrap();
    let Outcome::Completed(output) = outcome else {
        panic!("completed");
    };
    let data = output.data.expect("data");
    assert_eq!(data["source"], "legacy_secret");
    assert!(data["expires_at"].is_null());

    let held = f.vault.get(&super::account()).expect("kept");
    assert_eq!(held.source, TokenSource::LegacySecret);
    assert!(held.source.is_long_lived());
    assert!(held.text.expose().contains(TOKEN));
    assert!(!held.text.expose().contains("not-ours"));
    assert_eq!(held.warning.as_deref(), Some(LONG_LIVED_WARNING));
    assert!(
        held.text
            .expose()
            .contains("# WARNING: this token comes from")
    );
    assert!(
        held.preview.contains("# WARNING"),
        "the dialog shows it too"
    );
    assert_eq!(creates(&f), 0, "reading an existing Secret creates nothing");
}

#[test]
fn with_consent_and_no_secret_one_is_created_and_read_back() {
    let f = fixture();
    without_token_request(&f);
    // The token controller fills the Secret the create returned.
    f.h.resources(CLUSTER)
        .insert(legacy_secret("deployer-token-new", "deployer", Some(TOKEN)));
    f.h.resources(CLUSTER)
        .script()
        .list
        .push_ok(oxikube_ports::ListPage::complete(Vec::new()));
    f.h.resources(CLUSTER)
        .script()
        .create
        .push_ok(legacy_secret("deployer-token-new", "deployer", None));

    f.h.confirm_and_run(generate(None, true), ctx(Initiator::Ui))
        .unwrap();
    assert_eq!(creates(&f), 1);
    let created =
        f.h.writes(CLUSTER)
            .into_iter()
            .find_map(|call| match call {
                ResourceCall::Create { object, .. } => Some(object),
                _ => None,
            })
            .expect("a Secret was created");
    assert_eq!(created["type"], "kubernetes.io/service-account-token");
    assert_eq!(
        created["metadata"]["annotations"]["kubernetes.io/service-account.name"],
        "deployer"
    );
    let held = f.vault.get(&super::account()).expect("kept");
    assert_eq!(held.source, TokenSource::CreatedLegacySecret);
    assert!(held.warning.is_some());
}
