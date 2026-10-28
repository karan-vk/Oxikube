//! The command through the bus and the guard.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_ports::Subresource;
use oxikube_testkit::ResourceCall;
use serde_json::Value;

use super::{CLUSTER, TOKEN, account, fixture, generate, issued};
use crate::command_bus::{DispatchError, Outcome};
use crate::sa_kubeconfig::{TokenSource, format_lifetime};
use crate::testing::{ctx, id};

fn token_requests(f: &super::Fixture) -> Vec<ResourceCall> {
    f.h.writes(CLUSTER)
        .into_iter()
        .filter(|call| matches!(call, ResourceCall::CreateSubresource { .. }))
        .collect()
}

#[test]
fn it_is_a_low_risk_mutation_with_a_simple_confirmation_naming_the_account() {
    let f = fixture();
    let request = f.h.ask(generate(Some(7200), false), ctx(Initiator::Ui));
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.risk, Some(Risk::Low));
    assert!(
        request.summary.contains("shop/deployer") && request.summary.contains("2 hours"),
        "{}",
        request.summary
    );
    assert!(
        token_requests(&f).is_empty(),
        "nothing is sent before the confirmation"
    );
}

#[test]
fn a_token_request_becomes_a_kubeconfig_in_the_vault() {
    let f = fixture();
    f.h.resources(CLUSTER)
        .script()
        .create_subresource
        .push_ok(issued(TOKEN, "2026-10-28T13:00:00Z"));

    let outcome =
        f.h.confirm_and_run(generate(None, false), ctx(Initiator::Ui))
            .expect("runs");
    let Outcome::Completed(output) = outcome else {
        panic!("confirmed, so completed");
    };
    let data = output.data.expect("structured result");
    assert_eq!(data["outcome"], "generated");
    assert_eq!(data["source"], "token_request");
    assert_eq!(data["expires_at"], "2026-10-28T13:00:00Z");
    assert_eq!(data["context"], "deployer@a", "the new context's name");

    let calls = token_requests(&f);
    assert_eq!(calls.len(), 1);
    let ResourceCall::CreateSubresource {
        kind,
        namespace,
        name,
        subresource,
        body,
        options,
    } = &calls[0]
    else {
        unreachable!()
    };
    assert_eq!(kind.kind.as_ref(), "ServiceAccount");
    assert_eq!(namespace.as_deref(), Some("shop"));
    assert_eq!(name, "deployer");
    assert_eq!(subresource, &Subresource::Other("token".into()));
    assert_eq!(
        body["spec"]["expirationSeconds"], 3600,
        "the default is an hour"
    );
    assert!(!options.dry_run);

    let held = f.vault.get(&account()).expect("kept for the panel");
    assert_eq!(held.source, TokenSource::TokenRequest);
    assert_eq!(held.expires_at.as_deref(), Some("2026-10-28T13:00:00Z"));
    assert!(held.text.expose().contains(TOKEN));
    assert!(
        held.text
            .expose()
            .contains("server: https://a.example:6443")
    );
    assert!(!held.preview.contains(TOKEN), "the preview is masked");
    assert_eq!(held.file_name(), "shop-deployer-kubeconfig.yaml");
    assert!(held.warning.is_none());
    assert!(!format!("{held:?}").contains("sa-token-payload"));
}

#[test]
fn the_token_is_nowhere_but_the_vault() {
    let f = fixture();
    f.h.resources(CLUSTER)
        .script()
        .create_subresource
        .push_ok(issued(TOKEN, "2026-10-28T13:00:00Z"));
    let outcome =
        f.h.confirm_and_run(generate(Some(1800), false), ctx(Initiator::Agent))
            .expect("runs");
    let Outcome::Completed(output) = outcome else {
        panic!("completed");
    };
    let shown = format!("{output:?}{:?}", output.data.clone().unwrap_or(Value::Null));
    assert!(!shown.contains("sa-token-payload"), "the output: {shown}");
    let audit = format!("{:?}", f.h.audit());
    assert!(
        !audit.contains("sa-token-payload"),
        "the audit log: {audit}"
    );
    let requests = format!("{:?}", token_requests(&f));
    assert!(!requests.contains("sa-token-payload"));
}

#[test]
fn the_audit_record_names_the_account_and_the_lifetime() {
    let f = fixture();
    f.h.resources(CLUSTER)
        .script()
        .create_subresource
        .push_ok(issued(TOKEN, "2026-10-28T13:00:00Z"));
    f.h.confirm_and_run(generate(Some(1800), false), ctx(Initiator::Ui))
        .unwrap();
    let records = f.h.audit();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(&*record.cmd, "serviceaccount::GenerateKubeconfig");
    assert_eq!(record.outcome, AuditOutcome::Succeeded);
    assert_eq!(record.target.name.as_ref(), "deployer");
    assert_eq!(
        record.detail.as_deref(),
        Some("expiration=1800s audiences=0 legacy_allowed=false")
    );
}

#[test]
fn a_read_only_cluster_refuses_and_audits_the_denial() {
    let f = fixture();
    f.h.manager.set_read_only(&id(CLUSTER), true).ok();
    let denied = f.h.dispatch(generate(None, false), ctx(Initiator::Ui));
    assert!(
        matches!(denied, Err(DispatchError::ReadOnly { .. })),
        "{denied:?}"
    );
    assert!(token_requests(&f).is_empty());
    assert!(f.vault.is_empty());
    assert_eq!(f.h.outcomes(), [(AuditOutcome::Denied, Initiator::Ui)]);
}

#[test]
fn a_dry_run_changes_nothing_and_keeps_nothing() {
    let f = fixture();
    let request =
        f.h.ask(generate(None, false), ctx(Initiator::Ui).with_dry_run(true));
    let outcome = f.h.dispatch(
        generate(None, false),
        ctx(Initiator::Ui)
            .with_dry_run(true)
            .with_confirmation(crate::guard::Confirmation::simple(request.token)),
    );
    let Ok(Outcome::Completed(output)) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(output.data.expect("data")["outcome"], "dry_run");
    assert!(token_requests(&f).is_empty());
    assert!(f.vault.is_empty());
}

#[test]
fn a_lifetime_under_ten_minutes_is_refused_before_anything_is_sent() {
    let f = fixture();
    let error =
        f.h.confirm_and_run(generate(Some(60), false), ctx(Initiator::Ui));
    let Err(DispatchError::Handler(error)) = error else {
        panic!("{error:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(token_requests(&f).is_empty());
}

#[test]
fn a_missing_account_is_not_found_and_requests_no_token() {
    let f = fixture();
    let missing = Command::ServiceAccountGenerateKubeconfig {
        target: oxikube_domain::ids::ResourceRef::namespaced(
            id(CLUSTER),
            account().gvk,
            "shop",
            "ghost",
        ),
        expiration_seconds: None,
        audiences: Vec::new(),
        allow_legacy_token: false,
    };
    let error = f.h.confirm_and_run(missing, ctx(Initiator::Ui));
    let Err(DispatchError::Handler(error)) = error else {
        panic!("{error:?}");
    };
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert!(token_requests(&f).is_empty());
}

#[test]
fn another_kind_is_refused() {
    let f = fixture();
    let command = Command::ServiceAccountGenerateKubeconfig {
        target: crate::testing::pod(CLUSTER, "web"),
        expiration_seconds: None,
        audiences: Vec::new(),
        allow_legacy_token: false,
    };
    let error = f.h.confirm_and_run(command, ctx(Initiator::Ui));
    let Err(DispatchError::Handler(error)) = error else {
        panic!("{error:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Validation);
}

#[test]
fn a_refusal_by_the_server_is_passed_on() {
    let f = fixture();
    f.h.resources(CLUSTER).script().create_subresource.push_err(
        oxikube_domain::OxiError::forbidden("cannot create serviceaccounts/token"),
    );
    let error =
        f.h.confirm_and_run(generate(None, true), ctx(Initiator::Ui));
    let Err(DispatchError::Handler(error)) = error else {
        panic!("{error:?}");
    };
    assert_eq!(
        error.kind(),
        ErrorKind::Forbidden,
        "no fallback on a refusal"
    );
    assert!(f.vault.is_empty());
    assert_eq!(f.h.outcomes(), [(AuditOutcome::Failed, Initiator::Ui)]);
}

#[test]
fn it_has_a_tool_stub_and_the_registry_knows_it() {
    let id = CommandId::SERVICEACCOUNT_GENERATE_KUBECONFIG;
    assert_eq!(id.tool_name(), "k8s.serviceaccount_generate_kubeconfig");
    assert_eq!(format_lifetime(3600), "1 hour");
    assert_eq!(format_lifetime(600), "10 minutes");
    assert_eq!(format_lifetime(86_400 * 7), "7 days");
    assert_eq!(format_lifetime(90), "90 seconds");
}
