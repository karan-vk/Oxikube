//! Describing real objects on the kind cluster (`cargo xtask kind-up`):
//! `cargo test -p oxikube_describe --features integration --test kind_describe` with
//! `OXIKUBE_TEST_CONTEXT=kind-oxikube`. Skips when the variable is unset.
//!
//! The pod is made `Pending` with a scheduler nobody runs, so the shared cluster's scheduler pays
//! nothing for it; it lives in a namespace of its own, deleted when the test ends.

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::Arc;

use kube::Client;
use kube::config::{Config, KubeConfigOptions};
use oxikube_describe::{
    Backend, DescribeConfig, DescribePreference, Describer, KubectlDescribe, KubectlTarget,
    NativeDescribe,
};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_kube::discovery::KubeDiscovery;
use oxikube_ports::{DescribePort as _, DescribeSource};
use oxikube_testkit::integration::{TestNamespace, test_context};

fn apply(context: &str, namespace: &str, manifest: &str) {
    let mut child = Command::new("kubectl")
        .args(["--context", context, "-n", namespace, "apply", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("kubectl runs");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(manifest.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success(), "kubectl apply failed");
}

async fn client(context: &str) -> Client {
    let options = KubeConfigOptions {
        context: Some(context.to_owned()),
        ..KubeConfigOptions::default()
    };
    let config = Config::from_kubeconfig(&options).await.expect("kubeconfig");
    Client::try_from(config).expect("client")
}

fn pod_manifest() -> &'static str {
    r#"{"apiVersion":"v1","kind":"Pod","metadata":{"name":"describe-me","labels":{"app":"describe"}},
        "spec":{"schedulerName":"oxikube-test-nobody",
                "containers":[{"name":"app","image":"registry.invalid/never-pulled:1"}]}}"#
}

#[tokio::test]
async fn a_pod_on_kind_is_described_natively_and_by_kubectl() {
    let Some(context) = test_context() else {
        return;
    };
    let namespace = TestNamespace::create(&context).expect("namespace");
    apply(&context, namespace.name(), pod_manifest());

    let client = client(&context).await;
    let discovery = Arc::new(KubeDiscovery::new(client.clone()));
    let name = ContextName::new(&*context);
    let target = ResourceRef::namespaced(
        ClusterId::new("kind", &name),
        Gvk::new("", "v1", "Pod"),
        namespace.name(),
        "describe-me",
    );

    let native = NativeDescribe::new(client, discovery.clone());
    let output = native.describe(&target).await.expect("native describe");
    assert_eq!(output.source, DescribeSource::Native);
    assert!(output.text.contains("describe-me"), "{}", output.text);
    assert!(output.text.contains("Pending"), "{}", output.text);
    assert!(output.text.contains("app=describe"), "{}", output.text);

    // The same object through the CLI, selected by the setting.
    let preference = DescribePreference::new(DescribeConfig {
        backend: Backend::Kubectl,
        kubectl_path: None,
    });
    let kubectl = KubectlDescribe::new(
        discovery,
        preference.clone(),
        KubectlTarget {
            context: name,
            kubeconfig: None,
        },
    );
    let describer = Describer::new(Arc::new(native), Arc::new(kubectl), preference.clone());
    let output = describer.describe(&target).await.expect("kubectl describe");
    assert_eq!(output.source, DescribeSource::KubectlFallback);
    assert!(output.text.contains("describe-me"), "{}", output.text);

    // A pod that is not there: NotFound from either backend.
    let missing = ResourceRef::namespaced(
        target.cluster.clone(),
        Gvk::new("", "v1", "Pod"),
        namespace.name(),
        "not-there",
    );
    assert_eq!(
        describer.describe(&missing).await.unwrap_err().kind(),
        ErrorKind::NotFound
    );
    preference.set(DescribeConfig::default());
    assert_eq!(
        describer.describe(&missing).await.unwrap_err().kind(),
        ErrorKind::NotFound
    );
}
