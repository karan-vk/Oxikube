//! `cloud.discover_clusters` (E18-S08) over the fakes: schema, read-only, cache by default,
//! refresh on request, the in-catalog flag, no secrets, and one failing provider among three.

use std::sync::Arc;

use oxikube_domain::OxiError;
use oxikube_domain::command::{CloudRefreshMode, Command};
use oxikube_ports::{CloudProvider, ToolContext, ToolOutput};
use oxikube_testkit::{CloudCall, FakeCloudDiscoveryPort};
use serde_json::{Value, json};

use super::{Fixture, aws, aws_scope, eks};
use crate::tools::ToolRegistry;
use crate::tools::cloud::{DISCOVER_CLUSTERS, install};

fn registry(f: &Fixture) -> ToolRegistry {
    let tools = ToolRegistry::new();
    install(&tools, &f.service).unwrap();
    tools
}

async fn call(tools: &ToolRegistry, args: Value) -> ToolOutput {
    tools
        .invoke(DISCOVER_CLUSTERS, args, &ToolContext::agent())
        .await
        .expect("a result")
}

fn clusters(out: &ToolOutput) -> Vec<Value> {
    out.structured.as_ref().unwrap()["clusters"]
        .as_array()
        .unwrap()
        .clone()
}

fn list_calls(port: &FakeCloudDiscoveryPort) -> usize {
    port.recorded_calls()
        .iter()
        .filter(|c| matches!(c, CloudCall::ListClusters(_)))
        .count()
}

#[tokio::test]
async fn the_tool_is_registered_read_only_with_a_closed_schema() {
    let f = Fixture::new(vec![]);
    let tools = registry(&f);
    let defs = tools.defs();
    assert_eq!(defs.len(), 1);
    let def = &defs[0];
    def.validate().unwrap();
    assert_eq!(def.name.as_str(), DISCOVER_CLUSTERS);
    assert!(!def.is_mutating(), "reading clusters is not a mutation");
    assert!(def.risk.is_none());
    assert_eq!(def.input_schema["additionalProperties"], false);
    assert_eq!(def.input_schema["required"], json!(["provider"]));
}

#[tokio::test]
async fn arguments_are_validated() {
    let f = Fixture::new(vec![]);
    let tools = registry(&f);
    let ctx = ToolContext::agent();
    for bad in [
        json!({}),
        json!({"provider": "digitalocean"}),
        json!({"provider": "aws", "extra": 1}),
        json!({"provider": "aws", "refresh": "yes"}),
    ] {
        assert!(
            tools
                .invoke(DISCOVER_CLUSTERS, bad.clone(), &ctx)
                .await
                .is_err(),
            "{bad}"
        );
    }
}

#[tokio::test]
async fn it_answers_from_the_cache_and_refreshes_only_when_asked() {
    let prod = aws_scope("prod", "eu-west-1");
    let port = aws([prod.clone()], [eks(&prod, "web")]);
    let f = Fixture::new(vec![port.clone()]);
    let tools = registry(&f);

    let cold = call(&tools, json!({"provider": "aws"})).await;
    assert!(clusters(&cold).is_empty(), "nothing cached, nothing listed");
    assert_eq!(list_calls(&port), 0, "no CLI ran");

    let fresh = call(&tools, json!({"provider": "aws", "refresh": true})).await;
    assert_eq!(list_calls(&port), 1);
    assert_eq!(fresh.structured.as_ref().unwrap()["refreshed"], true);
    let rows = clusters(&fresh);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"], "web");
    assert_eq!(rows[0]["region"], "eu-west-1");
    assert_eq!(rows[0]["status"], "Running");
    assert_eq!(rows[0]["in_catalog"], false);

    let warm = call(&tools, json!({"provider": "aws", "refresh": false})).await;
    assert_eq!(clusters(&warm).len(), 1);
    assert_eq!(list_calls(&port), 1, "the cache answered");
}

#[tokio::test]
async fn the_in_catalog_flag_follows_add_and_remove() {
    let prod = aws_scope("prod", "eu-west-1");
    let web = eks(&prod, "web");
    let f = Fixture::new(vec![aws([prod.clone()], [web.clone(), eks(&prod, "api")])]);
    let tools = registry(&f);
    call(&tools, json!({"provider": "aws", "refresh": true})).await;
    f.service
        .execute(&Command::CloudAdd {
            provider: CloudProvider::Aws,
            cluster: web.key(),
        })
        .await
        .unwrap();

    let rows = clusters(&call(&tools, json!({"provider": "aws"})).await);
    let flags: Vec<(String, bool)> = rows
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap().to_owned(),
                r["in_catalog"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(flags, [("api".to_owned(), false), ("web".to_owned(), true)]);
}

#[tokio::test]
async fn results_carry_no_endpoint_ca_or_exec_arguments() {
    let prod = aws_scope("prod", "eu-west-1");
    let f = Fixture::new(vec![aws([prod.clone()], [eks(&prod, "web")])]);
    let tools = registry(&f);
    let out = call(&tools, json!({"provider": "aws", "refresh": true})).await;
    let text = format!("{:?}{:?}", out.structured, out.content);
    for secret in ["https://", "LS0tLS1", "get-token", "exec", "args", "token"] {
        assert!(!text.contains(secret), "{secret} leaked into {text}");
    }
    let row = &clusters(&out)[0];
    let mut keys: Vec<&str> = row
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["in_catalog", "name", "region", "status", "version"]);
}

#[tokio::test]
async fn a_failing_provider_among_three_keeps_the_others() {
    let prod = aws_scope("prod", "eu-west-1");
    let failing = Arc::new(FakeCloudDiscoveryPort::ready(CloudProvider::Gcp));
    let login = || OxiError::auth("run `gcloud auth login`", false);
    failing.script().scopes.push_err(login());
    let azure = Arc::new(FakeCloudDiscoveryPort::new(CloudProvider::Azure));
    let f = Fixture::new(vec![
        aws([prod.clone()], [eks(&prod, "web")]),
        failing.clone(),
        azure,
    ]);

    let output = f
        .service
        .execute(&Command::CloudRefresh {
            provider: None,
            mode: CloudRefreshMode::Force,
        })
        .await
        .expect("one provider listed, so the refresh succeeds");
    assert!(output.message.unwrap().contains("1 provider(s) failed"));

    let tools = registry(&f);
    assert_eq!(
        clusters(&call(&tools, json!({"provider": "aws"})).await).len(),
        1
    );
    failing.script().scopes.push_err(login());
    // The failing provider reports its error as a tool result, not a panic or a protocol error.
    let failed = call(&tools, json!({"provider": "gcloud", "refresh": true})).await;
    assert!(failed.is_error);
    assert!(format!("{:?}", failed.content).contains("gcloud auth login"));
    assert!(clusters(&call(&tools, json!({"provider": "az"})).await).is_empty());
}
