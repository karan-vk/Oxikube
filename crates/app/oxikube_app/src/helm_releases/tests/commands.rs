//! The three commands: masked answers, a reveal that carries a count and no value.

use oxikube_domain::command::Command;
use serde_json::json;

use super::{Fixture, cluster, values};
use crate::helm_releases::commands::run;

fn reveal(on: bool) -> Command {
    Command::HelmRevealValues {
        cluster: cluster(),
        namespace: "apps".into(),
        name: "web".into(),
        reveal: on,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn show_values_answers_masked_even_after_a_reveal() {
    let f = Fixture::new();
    for _ in 0..3 {
        f.helm.script().values.push_ok(values(
            json!({"db": {"password": "hunter2"}, "replicas": 3}),
        ));
    }
    let revealed = run(&f.service, reveal(true)).await.unwrap();
    let text = format!("{:?}", revealed.data);
    assert!(!text.contains("hunter2"), "a reveal reply carries no value");
    assert_eq!(revealed.data.as_ref().unwrap()["values"], 1);

    let shown = run(
        &f.service,
        Command::HelmShowValues {
            cluster: cluster(),
            namespace: "apps".into(),
            name: "web".into(),
            revision: Some(2),
            all: true,
        },
    )
    .await
    .unwrap();
    let data = shown.data.unwrap();
    let yaml = data["values"].as_str().unwrap();
    assert!(!yaml.contains("hunter2"));
    assert!(yaml.contains("replicas: 3"));
    assert_eq!(data["masked"], 1);
}

#[tokio::test(flavor = "current_thread")]
async fn show_manifest_masks_secrets() {
    let f = Fixture::new();
    f.helm.script().manifest.push_ok(
        "apiVersion: v1\nkind: Secret\nmetadata:\n  name: s\ndata:\n  k: c2VjcmV0\n".to_owned(),
    );
    let out = run(
        &f.service,
        Command::HelmShowManifest {
            cluster: cluster(),
            namespace: "apps".into(),
            name: "web".into(),
            revision: None,
        },
    )
    .await
    .unwrap();
    let data = out.data.unwrap();
    assert_eq!(data["secrets_masked"], true);
    assert!(!data["manifest"].as_str().unwrap().contains("c2VjcmV0"));
}

#[tokio::test(flavor = "current_thread")]
async fn other_commands_are_refused() {
    let f = Fixture::new();
    assert!(run(&f.service, Command::AppQuit).await.is_err());
}

#[test]
fn the_commands_are_read_only_and_declared_with_tool_stubs() {
    for id in crate::helm_releases::COMMANDS {
        let meta = oxikube_domain::command::lookup(id).expect("declared");
        assert!(meta.risk.is_none(), "{id} mutates nothing");
        assert!(id.tool_name().starts_with("helm."), "{}", id.tool_name());
    }
}
