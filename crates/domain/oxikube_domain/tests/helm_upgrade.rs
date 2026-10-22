//! E16-S10: the upgrade tab's pure parts: the objects an upgrade changes, the values text made
//! safe for a diff, and the commands.

use oxikube_domain::command::{Command, CommandId, lookup};
use oxikube_domain::helm::{ResourceChanges, ValuesYaml, mask_values_for_diff};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::safety::{ConfirmTier, Risk};
use serde_json::json;

const OLD: &str = "---\n# Source: a/svc.yaml\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n  namespace: demo\nspec:\n  ports:\n  - port: 80\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 1\n---\nkind: ConfigMap\nmetadata:\n  name: gone\n";

#[test]
fn objects_are_added_removed_and_changed_by_kind_and_name() {
    let new = "---\n# a different comment\nkind: Service\napiVersion: v1\nmetadata:\n  namespace: demo\n  name: web\nspec:\n  ports:\n    - port: 80\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 3\n---\nkind: Ingress\nmetadata:\n  name: web\n";
    let changes = ResourceChanges::between(OLD, new);
    let names = |keys: &[oxikube_domain::helm::ObjectKey]| {
        keys.iter().map(ToString::to_string).collect::<Vec<_>>()
    };
    assert_eq!(names(&changes.added), ["Ingress web"]);
    assert_eq!(names(&changes.removed), ["ConfigMap gone"]);
    assert_eq!(names(&changes.changed), ["Deployment web"]);
    // Comments, key order and indentation style are not changes.
    assert_eq!(changes.unchanged, 1);
    assert!(!changes.is_empty());
    assert_eq!(
        changes.describe(),
        "1 added, 1 removed, 1 changed, 1 unchanged"
    );
}

#[test]
fn the_same_manifest_changes_nothing_and_empty_sides_are_handled() {
    let same = ResourceChanges::between(OLD, OLD);
    assert!(same.is_empty());
    assert_eq!(same.unchanged, 3);
    assert_eq!(same.describe(), "No object changes (3 unchanged)");
    let all_new = ResourceChanges::between("", OLD);
    assert_eq!(all_new.added.len(), 3);
    let all_gone = ResourceChanges::between(OLD, "---\n---\n");
    assert_eq!(all_gone.removed.len(), 3);
    assert!(ResourceChanges::between("", "").is_empty());
}

#[test]
fn a_document_that_is_not_yaml_is_compared_as_text() {
    let changes = ResourceChanges::between("a: [\n", "a: [\n");
    assert_eq!(changes.unchanged, 1);
    let changes = ResourceChanges::between("a: [\n", "b: [\n");
    assert_eq!(changes.changed.len(), 1);
}

#[test]
fn secret_looking_values_are_fingerprinted_for_the_diff() {
    let text = "replicaCount: 2\nauth:\n  password: hunter2\n  user: admin\n  apiKey: abc\n  token: |\n    multi\nlist:\n  - secret: s3\nempty:\n  password:\n";
    let masked = mask_values_for_diff(text);
    assert!(!masked.contains("hunter2"), "{masked}");
    assert!(!masked.contains("abc"), "{masked}");
    assert!(!masked.contains("s3"), "{masked}");
    assert!(masked.contains("user: admin"));
    assert!(masked.contains("replicaCount: 2"));
    assert!(masked.contains("password: (hidden #"), "{masked}");
    assert!(masked.contains("- secret: (hidden #"), "{masked}");
    // Equal secrets, equal fingerprints; a changed secret shows as a change.
    let again = mask_values_for_diff("auth:\n  password: hunter2\n");
    let other = mask_values_for_diff("auth:\n  password: hunter3\n");
    assert!(masked.contains(again.lines().nth(1).unwrap()));
    assert_ne!(again, other);
}

#[test]
fn upgrade_is_a_medium_mutation_with_a_tool_and_carries_no_values_in_debug() {
    let meta = lookup(CommandId::HELM_UPGRADE).expect("declared");
    assert!(meta.mutating);
    assert_eq!(meta.risk, Some(Risk::Medium));
    assert_eq!(meta.confirm, ConfirmTier::Simple);
    assert_eq!(CommandId::HELM_UPGRADE.tool_name(), "helm.upgrade");
    let cluster = ClusterId::new("/k", &ContextName::new("kind"));
    let command = Command::HelmUpgrade {
        cluster,
        namespace: "demo".into(),
        release: "web".into(),
        repo: Some("bitnami".into()),
        chart: "nginx".into(),
        version: Some("15.5.0".into()),
        values: ValuesYaml::new("auth:\n  password: s3cret\n"),
        reuse_values: true,
        wait: false,
        timeout_secs: None,
    };
    assert!(command.is_mutating());
    assert!(!format!("{command:?}").contains("s3cret"));
    let back: Command = serde_json::from_value(serde_json::to_value(&command).unwrap()).unwrap();
    assert_eq!(back, command);
    let minimal: Command = serde_json::from_value(json!({
        "type": "helm::Upgrade",
        "cluster": serde_json::to_value(ClusterId::new("/k", &ContextName::new("kind"))).unwrap(),
        "namespace": "demo",
        "release": "web",
        "chart": "nginx"
    }))
    .unwrap();
    assert!(matches!(
        minimal,
        Command::HelmUpgrade {
            reuse_values: false,
            wait: false,
            version: None,
            ..
        }
    ));
}

#[test]
fn opening_the_upgrade_tab_is_a_read_that_may_name_no_release() {
    let meta = lookup(CommandId::HELM_UPGRADE_RELEASE).expect("declared");
    assert!(!meta.mutating);
    let open: Command = serde_json::from_value(json!({ "type": "helm::UpgradeRelease" })).unwrap();
    assert_eq!(
        open,
        Command::HelmUpgradeRelease {
            namespace: None,
            release: None
        }
    );
    assert!(!open.is_mutating());
}
