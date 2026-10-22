use super::*;

const MULTI: &str = r#"---
# Source: web/templates/sa.yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: web
---
# Source: web/templates/deploy.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web
  namespace: apps
spec:
  replicas: 2
---

---
# only a comment
---
# Source: web/templates/crb.yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: web
"#;

fn names(objects: &[ManifestObject]) -> Vec<String> {
    objects
        .iter()
        .map(|o| format!("{}/{}", o.kind, o.name))
        .collect()
}

#[test]
fn splits_documents_and_skips_empty_and_comment_only_ones() {
    let parsed = parse_manifest(MULTI);
    assert_eq!(
        names(&parsed.objects),
        [
            "ServiceAccount/web",
            "Deployment/web",
            "ClusterRoleBinding/web"
        ]
    );
    assert!(parsed.errors.is_empty() && parsed.hooks.is_empty());
}

#[test]
fn reads_group_version_namespace_and_source() {
    let parsed = parse_manifest(MULTI);
    let deploy = &parsed.objects[1];
    assert_eq!((deploy.group(), deploy.version()), ("apps", "v1"));
    assert_eq!(deploy.namespace.as_deref(), Some("apps"));
    assert_eq!(deploy.source.as_deref(), Some("web/templates/deploy.yaml"));
    let sa = &parsed.objects[0];
    assert_eq!((sa.group(), sa.version()), ("", "v1"));
    assert_eq!(sa.namespace, None);
}

#[test]
fn hooks_are_listed_apart() {
    let parsed = parse_manifest(
        "apiVersion: batch/v1\nkind: Job\nmetadata:\n  name: migrate\n  annotations:\n    helm.sh/hook: pre-install\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n",
    );
    assert_eq!(names(&parsed.hooks), ["Job/migrate"]);
    assert_eq!(names(&parsed.objects), ["Service/web"]);
}

#[test]
fn a_broken_document_is_reported_and_the_rest_survive() {
    let parsed = parse_manifest(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\n---\nkey: [unclosed secret-value\n  : : :\n---\nkind: Thing\n---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: b\n",
    );
    assert_eq!(names(&parsed.objects), ["ConfigMap/a", "ConfigMap/b"]);
    assert_eq!(parsed.errors.len(), 2);
    assert_eq!(parsed.errors[0].document, 2);
    assert_eq!(parsed.errors[1].document, 3);
    for error in &parsed.errors {
        assert!(!error.message.contains("secret-value"), "{error:?}");
    }
}

#[test]
fn a_list_document_contributes_its_items() {
    let parsed = parse_manifest(
        "apiVersion: v1\nkind: List\nitems:\n- apiVersion: v1\n  kind: ConfigMap\n  metadata:\n    name: a\n- apiVersion: v1\n  kind: Secret\n  metadata:\n    name: b\n",
    );
    assert_eq!(names(&parsed.objects), ["ConfigMap/a", "Secret/b"]);
}

#[test]
fn block_scalars_containing_dashes_do_not_split() {
    let parsed = parse_manifest(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\ndata:\n  script: |\n    ---\n    echo hi\n",
    );
    assert_eq!(names(&parsed.objects), ["ConfigMap/a"]);
    assert!(parsed.errors.is_empty());
}

#[test]
fn empty_input_is_empty() {
    assert_eq!(parse_manifest(""), ParsedManifest::default());
    assert_eq!(
        parse_manifest("---\n---\n# nothing\n"),
        ParsedManifest::default()
    );
}
