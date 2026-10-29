//! Copy to another namespace (E20-S08) against the real adapter: `CopyService` through the
//! `CommandBus` and `MutationGuard` on a kind cluster, no UI. The two "clusters" are two
//! namespaces of the one shared cluster (a second real cluster adds nothing the app layer can
//! see: the source and the target are each a session with its own guard).
//!
//! * a ConfigMap and a Deployment are copied: the server's fields (uid, resourceVersion, status,
//!   the last-applied annotation) are gone, the labels stay, the name can change;
//! * an object that exists is previewed as an overwrite, copying again over it is a no-op write,
//!   a changed source is configured, Skip writes nothing;
//! * a namespace that does not exist is refused; a read-only target is refused with zero writes;
//! * both audit logs get a record.
//!
//! The two namespaces have unique names and are deleted when the test ends, also when it panics;
//! the Deployment has no replicas, so nothing is scheduled.

use std::process::Command;
use std::sync::Arc;

use oxikube_app::command_bus::CommandRegistry;
use oxikube_app::copy::{
    CopyChoice, CopyError, CopyOutcome, CopyRequest, CopyService, ReviewAnswer, register_commands,
};
use oxikube_app::session::{ClusterSessionManager, SessionManagerConfig, SessionOptions};
use oxikube_app::{CommandBus, MutationGuard};
use oxikube_domain::audit::AuditOutcome;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_kube::{ConnectorConfig, KubeConnector, PoolConfig};
use oxikube_testkit::integration::random_namespace_name;
use oxikube_testkit::{FakeClusterSourcePort, FakeStatePort};
use serde_json::Value;

use crate::clock::TokioClock;
use crate::cluster::{Kind, catalog_entry};

/// Deletes the test's namespaces (best effort, no wait) when dropped.
struct Cleanup {
    context: String,
    namespaces: Vec<String>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        for namespace in &self.namespaces {
            let _ = Command::new("kubectl")
                .args(["--context", &self.context, "delete", "namespace", namespace])
                .args(["--ignore-not-found", "--wait=false"])
                .output();
        }
    }
}

fn kubectl(context: &str, args: &[&str], stdin: Option<&str>) -> String {
    use std::io::Write as _;
    use std::process::Stdio;
    let mut child = Command::new("kubectl")
        .args(["--context", context])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("kubectl runs");
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(text.as_bytes())
            .expect("write the manifest");
    }
    let output = child.wait_with_output().expect("kubectl finishes");
    assert!(
        output.status.success(),
        "kubectl {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn get(context: &str, namespace: &str, what: &str) -> Value {
    let text = kubectl(context, &["-n", namespace, "get", what, "-o", "json"], None);
    serde_json::from_str(&text).expect("kubectl prints JSON")
}

struct Rig {
    service: CopyService,
    manager: ClusterSessionManager,
    state: Arc<FakeStatePort>,
    cluster: oxikube_domain::ids::ClusterId,
}

impl Rig {
    async fn new(kind: &Kind) -> Self {
        let entry = catalog_entry(&kind.context);
        let connector = KubeConnector::new(
            kind.kubeconfig.clone(),
            PoolConfig::default(),
            ConnectorConfig::default(),
        );
        let manager = ClusterSessionManager::with_config(
            Arc::new(connector),
            Arc::new(FakeClusterSourcePort::new()),
            Arc::new(TokioClock),
            SessionManagerConfig::default(),
        );
        let state = Arc::new(FakeStatePort::new());
        let mut registry = CommandRegistry::new();
        let sessions = manager.clone();
        registry
            .install("oxikube_app::copy", move |r| register_commands(r, sessions))
            .expect("register the copy command");
        let bus = CommandBus::new(
            registry,
            MutationGuard::new(manager.clone(), state.clone(), Arc::new(TokioClock)),
        );
        manager.open(&entry, SessionOptions::default());
        manager.connect(&entry.cluster).await.expect("connect");
        Self {
            service: CopyService::new(bus, manager.clone(), "tester"),
            manager,
            state,
            cluster: entry.cluster,
        }
    }

    fn request(&self, gvk: Gvk, from: &str, to: &str, name: &str) -> CopyRequest {
        let source = ResourceRef::namespaced(self.cluster.clone(), gvk, from, name);
        CopyRequest::to(source, self.cluster.clone(), Some(to))
    }
}

#[tokio::test]
async fn an_object_is_copied_to_another_namespace_and_both_logs_say_so() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let context = kind.context.to_string();
    let (from, to) = (random_namespace_name(), random_namespace_name());
    let _cleanup = Cleanup {
        context: context.clone(),
        namespaces: vec![from.clone(), to.clone()],
    };
    for namespace in [&from, &to] {
        kubectl(&context, &["create", "namespace", namespace], None);
    }
    kubectl(
        &context,
        &["-n", &from, "apply", "-f", "-"],
        Some(&format!(
            "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: settings\n  labels:\n    tier: backend\ndata:\n  mode: blue\n---\n\
             apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 0\n  selector:\n    matchLabels:\n      app: web\n  template:\n    metadata:\n      labels:\n        app: web\n    spec:\n      containers:\n      - name: web\n        image: {}\n",
            oxikube_testkit::images::PAUSE
        )),
    );
    let rig = Rig::new(&kind).await;
    let configmap = Gvk::from_api_version("v1", "ConfigMap");
    let deployment = Gvk::from_api_version("apps/v1", "Deployment");

    // A ConfigMap: previewed (a dry run, nothing written), then copied.
    let preview = rig
        .service
        .preview(rig.request(configmap.clone(), &from, &to, "settings"))
        .await
        .expect("preview");
    assert!(!preview.exists());
    assert!(
        preview.removed.iter().any(|path| path == "metadata.uid"),
        "{:?}",
        preview.removed
    );
    let listed = kubectl(
        &context,
        &["-n", &to, "get", "configmap", "-o", "name"],
        None,
    );
    assert!(
        !listed.contains("settings"),
        "a preview writes nothing: {listed}"
    );
    let outcome = rig
        .service
        .copy(&preview, None, ReviewAnswer::click())
        .await
        .expect("copy");
    assert!(
        matches!(outcome, CopyOutcome::Copied { created: true, .. }),
        "{outcome:?}"
    );
    let copied = get(&context, &to, "configmap/settings");
    assert_eq!(copied["data"]["mode"], "blue");
    assert_eq!(copied["metadata"]["labels"]["tier"], "backend");
    assert_eq!(copied["metadata"]["namespace"], to.as_str());
    let original = get(&context, &from, "configmap/settings");
    assert_ne!(copied["metadata"]["uid"], original["metadata"]["uid"]);

    // A Deployment, renamed: the selector is kept, the server's fields and status are not.
    let request = rig
        .request(deployment.clone(), &from, &to, "web")
        .renamed("web-copy");
    let preview = rig.service.preview(request).await.expect("preview");
    rig.service
        .copy(&preview, None, ReviewAnswer::click())
        .await
        .expect("copy");
    let web = get(&context, &to, "deployment/web-copy");
    assert_eq!(web["spec"]["selector"]["matchLabels"]["app"], "web");
    assert!(
        web["metadata"]["annotations"]
            .get("kubectl.kubernetes.io/last-applied-configuration")
            .is_none()
    );

    // It exists now: previewed as an overwrite; no choice, no write.
    let preview = rig
        .service
        .preview(rig.request(configmap.clone(), &from, &to, "settings"))
        .await
        .expect("preview");
    assert!(preview.exists());
    assert!(matches!(
        rig.service
            .copy(&preview, None, ReviewAnswer::click())
            .await,
        Err(CopyError::Exists(_))
    ));
    // Skip writes nothing; Overwrite of an identical object is a no-op; a changed source is
    // configured; Rename copies under the new name.
    let skipped = rig
        .service
        .copy(&preview, Some(CopyChoice::Skip), ReviewAnswer::click())
        .await
        .expect("skip");
    assert!(matches!(skipped, CopyOutcome::Skipped { .. }));
    let version = |value: &Value| value["metadata"]["resourceVersion"].clone();
    let before = version(&get(&context, &to, "configmap/settings"));
    rig.service
        .copy(
            &preview,
            Some(CopyChoice::Overwrite { force: false }),
            ReviewAnswer::click(),
        )
        .await
        .expect("overwrite");
    assert_eq!(version(&get(&context, &to, "configmap/settings")), before);
    kubectl(
        &context,
        &[
            "-n",
            &from,
            "patch",
            "configmap",
            "settings",
            "--type=merge",
            "-p",
            r#"{"data":{"mode":"green"}}"#,
        ],
        None,
    );
    let preview = rig
        .service
        .preview(rig.request(configmap.clone(), &from, &to, "settings"))
        .await
        .expect("preview");
    assert_eq!(preview.change(), oxikube_app::apply::Change::Update);
    rig.service
        .copy(
            &preview,
            Some(CopyChoice::Overwrite { force: false }),
            ReviewAnswer::click(),
        )
        .await
        .expect("overwrite");
    assert_eq!(
        get(&context, &to, "configmap/settings")["data"]["mode"],
        "green"
    );
    rig.service
        .copy(
            &preview,
            Some(CopyChoice::Rename("settings-copy".into())),
            ReviewAnswer::click(),
        )
        .await
        .expect("rename");
    get(&context, &to, "configmap/settings-copy");

    // A namespace that does not exist is refused.
    let error = rig
        .service
        .preview(rig.request(configmap.clone(), &from, "oxi-test-nowhere", "settings"))
        .await
        .unwrap_err();
    assert!(matches!(error, CopyError::NamespaceMissing(_)), "{error:?}");

    // Both logs: the target's mutation and the source's read.
    let records = rig.state.audit_log();
    let real: Vec<_> = records
        .iter()
        .filter(|r| r.cmd.as_ref() == "resource::CopyTo" && !r.dry_run)
        .collect();
    assert!(real.iter().all(|r| r.outcome == AuditOutcome::Succeeded));
    assert!(real.iter().any(|r| {
        r.detail
            .as_deref()
            .is_some_and(|d| d.contains("copied_from=ConfigMap"))
    }));
    assert!(real.iter().any(|r| {
        r.detail
            .as_deref()
            .is_some_and(|d| d.contains("copied_to=ConfigMap"))
    }));

    // A read-only target is refused before anything is read or written.
    rig.manager
        .set_read_only(&rig.cluster, true)
        .expect("read-only");
    let refused = rig
        .service
        .preview(rig.request(configmap, &from, &to, "settings"))
        .await;
    assert!(
        matches!(refused, Err(CopyError::ReadOnly { .. })),
        "{refused:?}"
    );
}
