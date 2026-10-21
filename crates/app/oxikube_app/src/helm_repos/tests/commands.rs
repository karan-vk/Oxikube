use std::sync::Arc;

use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::safety::Initiator;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_testkit::{FakeHelmPort, HelmCall};

use super::{entry, service};
use crate::helm_repos::commands::run;
use crate::helm_repos::{COMMANDS, DenyAgentRepoAdd, RepoPermission};

fn add(name: &str) -> Command {
    Command::HelmRepoAdd {
        name: name.into(),
        url: format!("https://{name}.example.com"),
        username: Some("bob".into()),
        insecure_skip_tls_verify: true,
        ca_file: Some("/ca.pem".into()),
    }
}

#[tokio::test]
async fn the_add_command_builds_the_request_and_runs_it() {
    let fake = Arc::new(FakeHelmPort::new());
    let service = service(&fake);
    let out = run(&service, &DenyAgentRepoAdd, Initiator::Ui, add("bitnami"))
        .await
        .unwrap();
    assert_eq!(out.message.as_deref(), Some("Added repository bitnami"));
    let HelmCall::RepoAdd(request) = &fake.recorded_calls()[0] else {
        panic!("add first");
    };
    assert_eq!(request.username.as_deref(), Some("bob"));
    assert!(request.insecure_skip_tls_verify);
    assert_eq!(request.ca_file.as_deref(), Some("/ca.pem"));
}

#[tokio::test]
async fn an_agent_needs_permission_to_add_a_repository() {
    let fake = Arc::new(FakeHelmPort::new());
    let service = service(&fake);
    let error = run(&service, &DenyAgentRepoAdd, Initiator::Agent, add("evil"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(fake.recorded_calls().is_empty());

    struct Allow;
    #[async_trait::async_trait]
    impl RepoPermission for Allow {
        async fn allows_agent(&self, _: &Command) -> bool {
            true
        }
    }
    run(&service, &Allow, Initiator::Agent, add("ok"))
        .await
        .unwrap();
    // Removing needs it too; refreshing what the user chose does not.
    let remove = || Command::HelmRepoRemove { name: "ok".into() };
    let refused = run(&service, &DenyAgentRepoAdd, Initiator::Agent, remove()).await;
    assert_eq!(refused.unwrap_err().kind(), ErrorKind::Forbidden);
    run(&service, &Allow, Initiator::Agent, remove())
        .await
        .unwrap();
    run(
        &service,
        &DenyAgentRepoAdd,
        Initiator::Agent,
        Command::HelmRepoUpdate {
            names: vec!["a".into()],
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_update_command_reports_each_repository() {
    let fake = Arc::new(FakeHelmPort::new());
    // Read by `refresh`, then again by the update of "every repository".
    fake.script().repos.push_ok(vec![entry("a"), entry("b")]);
    fake.script().repos.push_ok(vec![entry("a"), entry("b")]);
    fake.script().repo_update.push_ok(());
    fake.script()
        .repo_update
        .push_err(OxiError::internal("dial tcp"));
    let service = service(&fake);
    service.refresh().await.unwrap();
    let out = run(
        &service,
        &DenyAgentRepoAdd,
        Initiator::Command,
        Command::HelmRepoUpdate { names: vec![] },
    )
    .await
    .unwrap();
    let data = out.data.unwrap();
    assert_eq!(data["updated"], serde_json::json!(["a"]));
    assert_eq!(data["failed"][0]["name"], "b");
}

#[test]
fn every_command_is_declared_with_a_tool_stub() {
    for id in COMMANDS {
        let meta = oxikube_domain::command::lookup(id).expect("declared");
        assert!(
            !meta.mutating,
            "{id}: local configuration, not a cluster mutation"
        );
        assert!(id.tool_name().starts_with("helm."), "{id} {}", meta.title);
    }
    assert_eq!(CommandId::HELM_REPO_ADD.tool_name(), "helm.repo_add");
}
