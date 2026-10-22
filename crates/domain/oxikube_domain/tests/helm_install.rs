//! E16-S09: the install tab's pure parts: name rules, the values text, the manifest summary and
//! the values documentation (helm-docs comments, plain comments, `values.schema.json`).

use oxikube_domain::command::{Command, CommandId, lookup};
use oxikube_domain::helm::{
    MAX_RELEASE_NAME, ManifestSummary, ValuesDocs, ValuesYaml, validate_namespace_name,
    validate_release_name,
};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::safety::{ConfirmTier, Risk};
use serde_json::json;

#[test]
fn release_names_follow_helms_rules() {
    assert!(validate_release_name("web").is_ok());
    assert!(validate_release_name("my-app-2").is_ok());
    assert!(validate_release_name(&"a".repeat(MAX_RELEASE_NAME)).is_ok());
    let too_long = validate_release_name(&"a".repeat(MAX_RELEASE_NAME + 1)).unwrap_err();
    assert!(too_long.contains("53"), "{too_long}");
    assert!(validate_release_name("").is_err());
    assert!(validate_release_name("Web").is_err());
    assert!(validate_release_name("web_app").is_err());
    assert!(validate_release_name("web.app").is_err());
    assert!(validate_release_name("-web").is_err());
    assert!(validate_release_name("web-").is_err());
}

#[test]
fn namespace_names_are_dns_labels_of_63() {
    assert!(validate_namespace_name("kube-system").is_ok());
    assert!(validate_namespace_name(&"n".repeat(63)).is_ok());
    assert!(validate_namespace_name(&"n".repeat(64)).is_err());
    assert!(validate_namespace_name("Default").is_err());
}

#[test]
fn values_text_never_prints_and_must_be_a_map() {
    let values = ValuesYaml::new("password: hunter2\n");
    assert!(!format!("{values:?}").contains("hunter2"));
    assert_eq!(
        values.parse().unwrap().get("password"),
        Some(&json!("hunter2"))
    );
    assert!(ValuesYaml::new("   \n").is_blank());
    assert!(ValuesYaml::new("").parse().unwrap().is_empty());
    assert!(ValuesYaml::new("- a\n- b\n").parse().is_err());
    assert!(ValuesYaml::new("a: [\n").parse().is_err());
    assert_eq!(
        serde_json::to_value(&values).unwrap(),
        json!("password: hunter2\n")
    );
}

#[test]
fn install_is_a_low_risk_mutation_with_a_tool_and_carries_no_values_in_debug() {
    let meta = lookup(CommandId::HELM_INSTALL).expect("declared");
    assert!(meta.mutating);
    assert_eq!(meta.risk, Some(Risk::Low));
    assert_eq!(meta.confirm, ConfirmTier::Simple);
    assert_eq!(CommandId::HELM_INSTALL.tool_name(), "helm.install");
    let cluster = ClusterId::new("/k", &ContextName::new("kind"));
    let command = Command::HelmInstall {
        cluster,
        release: "web".into(),
        namespace: "demo".into(),
        create_namespace: true,
        repo: Some("bitnami".into()),
        chart: "nginx".into(),
        version: None,
        values: ValuesYaml::new("auth:\n  password: s3cret\n"),
        wait: false,
        timeout_secs: None,
    };
    assert!(command.is_mutating());
    assert!(!format!("{command:?}").contains("s3cret"));
    let back: Command = serde_json::from_value(serde_json::to_value(&command).unwrap()).unwrap();
    assert_eq!(back, command);
    // A tool call with only the essentials is a valid install of the chart's defaults.
    let minimal: Command = serde_json::from_value(json!({
        "type": "helm::Install",
        "cluster": serde_json::to_value(command_cluster(&command)).unwrap(),
        "release": "web", "namespace": "demo", "chart": "nginx"
    }))
    .unwrap();
    assert!(matches!(minimal, Command::HelmInstall { values, .. } if values.is_blank()));
}

fn command_cluster(command: &Command) -> &ClusterId {
    match command {
        Command::HelmInstall { cluster, .. } => cluster,
        _ => unreachable!(),
    }
}

#[test]
fn manifest_summary_counts_objects_by_kind() {
    let manifest = "---\n# Source: nginx/templates/svc.yaml\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n---\napiVersion: apps/v1\nkind: Deployment\nspec:\n  template:\n    spec:\n      containers:\n      - name: x\n        kind: nested-not-counted\n---\n\n---\napiVersion: v1\nkind: \"Service\"\n";
    let summary = ManifestSummary::of(manifest);
    assert_eq!(summary.kinds.get("Service"), Some(&2));
    assert_eq!(summary.kinds.get("Deployment"), Some(&1));
    assert_eq!(summary.total(), 3);
    assert_eq!(summary.describe(), "3 objects: 2 Service, 1 Deployment");
    assert_eq!(ManifestSummary::of("").describe(), "No objects");
}

const VALUES: &str = r#"# Default values for nginx.
# This is a YAML-formatted file.

# -- Number of replicas
replicaCount: 1

image:
  # -- (string) Image repository
  # on Docker Hub
  repository: bitnami/nginx
  # -- Overrides the image tag.
  # @default -- the chart's appVersion
  tag: ""
  # Plain comment for the policy
  pullPolicy: IfNotPresent # inline

# service.port -- The port the Service listens on
service:
  type: ClusterIP
  port: 80
  annotations: {}

config: |
  key: not-a-values-key
  other: also-not
auth:
  password: ""
extraEnv:
  - name: FOO
    value: bar
"quoted.key": 1
"#;

#[test]
fn helm_docs_comments_document_their_keys() {
    let docs = ValuesDocs::from_values_yaml(VALUES);
    let replicas = docs.get("replicaCount").unwrap();
    assert_eq!(replicas.description, "Number of replicas");
    assert_eq!(replicas.type_hint.as_deref(), Some("int"));
    assert_eq!(replicas.default.as_deref(), Some("1"));

    let repo = docs.get("image.repository").unwrap();
    assert_eq!(repo.description, "Image repository\non Docker Hub");
    assert_eq!(repo.type_hint.as_deref(), Some("string"));

    let tag = docs.get("image.tag").unwrap();
    assert_eq!(tag.description, "Overrides the image tag.");
    assert_eq!(tag.default.as_deref(), Some("the chart's appVersion"));

    let policy = docs.get("image.pullPolicy").unwrap();
    assert_eq!(policy.description, "Plain comment for the policy");
    assert_eq!(policy.default.as_deref(), Some("IfNotPresent"));

    assert_eq!(
        docs.get("service.port").unwrap().description,
        "The port the Service listens on"
    );
    assert_eq!(
        docs.get("service.annotations")
            .unwrap()
            .type_hint
            .as_deref(),
        Some("object")
    );
    // The file's header comment is separated by a blank line: it documents nothing.
    assert!(docs.get("image").unwrap().description.is_empty());
}

#[test]
fn block_scalars_and_list_items_are_not_keys() {
    let docs = ValuesDocs::from_values_yaml(VALUES);
    assert!(docs.get("config").is_some());
    assert!(docs.get("config.key").is_none());
    assert!(docs.get("key").is_none());
    assert!(docs.get("auth.password").is_some());
    assert!(docs.get("extraEnv").is_some());
    assert!(docs.get("extraEnv.name").is_none());
    assert!(docs.get("name").is_none());
    assert!(docs.get("quoted.key").is_some());
}

#[test]
fn keys_continuing_a_list_item_are_not_keys_of_the_enclosing_map() {
    let docs = ValuesDocs::from_values_yaml(
        "env:\n  - name: A\n    value: b\n    # -- not a key\n    nested:\n      deep: 1\n  - name: B\n    value: c\ntolerations:\n- key: a\n  operator: Exists\nafter:\n  inner: 2\n",
    );
    for bogus in [
        "env.name",
        "env.value",
        "env.nested",
        "env.nested.deep",
        "tolerations.key",
        "tolerations.operator",
        "value",
        "operator",
    ] {
        assert!(docs.get(bogus).is_none(), "{bogus}");
    }
    for key in ["env", "tolerations", "after", "after.inner"] {
        assert!(docs.get(key).is_some(), "{key}");
    }
    assert_eq!(docs.children("env").count(), 0);
}

#[test]
fn children_are_the_direct_keys_of_a_map() {
    let docs = ValuesDocs::from_values_yaml(VALUES);
    let top: Vec<_> = docs.children("").map(|(k, _)| k).collect();
    assert!(top.contains(&"image") && top.contains(&"replicaCount") && top.contains(&"service"));
    assert!(!top.contains(&"tag"));
    let image: Vec<_> = docs.children("image").map(|(k, _)| k).collect();
    assert_eq!(image, ["pullPolicy", "repository", "tag"]);
}

#[test]
fn the_schema_fills_what_the_comments_left_out() {
    let mut docs = ValuesDocs::from_values_yaml(VALUES);
    docs.merge_schema(&json!({
        "type": "object",
        "properties": {
            "replicaCount": { "type": "integer", "description": "ignored: the comment wins" },
            "service": {
                "type": "object",
                "properties": {
                    "type": {
                        "type": "string",
                        "description": "How the Service is exposed",
                        "enum": ["ClusterIP", "NodePort", "LoadBalancer"]
                    }
                }
            },
            "newKey": { "type": ["string", "null"], "description": "Only in the schema" }
        }
    }));
    let replicas = docs.get("replicaCount").unwrap();
    assert_eq!(replicas.description, "Number of replicas");
    assert_eq!(replicas.type_hint.as_deref(), Some("integer"));
    let service_type = docs.get("service.type").unwrap();
    assert_eq!(service_type.description, "How the Service is exposed");
    assert_eq!(
        service_type.allowed,
        ["ClusterIP", "NodePort", "LoadBalancer"]
    );
    assert_eq!(
        docs.get("newKey").unwrap().type_hint.as_deref(),
        Some("string | null")
    );
    let hover = docs.hover_markdown("service.type").unwrap();
    assert!(hover.contains("**service.type** `string`"), "{hover}");
    assert!(hover.contains("Default: `ClusterIP`"), "{hover}");
    assert!(
        hover.contains("One of: ClusterIP, NodePort, LoadBalancer"),
        "{hover}"
    );
    assert!(docs.hover_markdown("nope").is_none());
}

#[test]
fn a_large_values_file_is_one_pass() {
    let mut text = String::new();
    for i in 0..4000 {
        text.push_str(&format!(
            "# -- key {i}\nkey{i}:\n  nested: {i}\n  other: \"x\"\n"
        ));
    }
    assert!(text.len() > 200_000);
    let started = std::time::Instant::now();
    let docs = ValuesDocs::from_values_yaml(&text);
    assert_eq!(docs.len(), 12_000);
    assert_eq!(docs.get("key3999").unwrap().description, "key 3999");
    // Generous for debug builds and CI: the point is linear, not quadratic.
    assert!(started.elapsed().as_millis() < 2_000);
}
