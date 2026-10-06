//! Choosing the backend: the preference, and the fallback of `auto`.

use std::sync::Arc;

use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{DescribeOutput, DescribeSource};
use oxikube_testkit::FakeDescribePort;

use super::pod_ref;
use crate::{Backend, DescribeConfig, DescribePreference, Describer};
use oxikube_ports::DescribePort as _;

fn out(text: &str, source: DescribeSource) -> DescribeOutput {
    DescribeOutput {
        text: text.into(),
        source,
    }
}

struct Setup {
    native: Arc<FakeDescribePort>,
    kubectl: Arc<FakeDescribePort>,
    preference: DescribePreference,
    describer: Describer,
}

fn setup(backend: Backend) -> Setup {
    let native = Arc::new(FakeDescribePort::new());
    let kubectl = Arc::new(FakeDescribePort::new());
    let preference = DescribePreference::new(DescribeConfig {
        backend,
        kubectl_path: None,
    });
    let describer = Describer::new(native.clone(), kubectl.clone(), preference.clone());
    Setup {
        native,
        kubectl,
        preference,
        describer,
    }
}

#[tokio::test]
async fn auto_uses_the_native_renderer_first() {
    let s = setup(Backend::Auto);
    s.native
        .script()
        .describe
        .push_ok(out("native text", DescribeSource::Native));
    let got = s.describer.describe(&pod_ref()).await.unwrap();
    assert_eq!(got.text, "native text");
    assert!(s.kubectl.recorded_calls().is_empty());
}

#[tokio::test]
async fn auto_falls_back_to_kubectl_for_a_kind_the_native_renderer_does_not_cover() {
    let s = setup(Backend::Auto);
    s.native.script().describe.push_err(OxiError::unsupported(
        "native describe does not cover Gadget",
    ));
    s.kubectl
        .script()
        .describe
        .push_ok(out("kubectl text", DescribeSource::KubectlFallback));
    let got = s.describer.describe(&pod_ref()).await.unwrap();
    assert_eq!(got.source, DescribeSource::KubectlFallback);
}

#[tokio::test]
async fn auto_does_not_retry_a_real_failure_with_kubectl() {
    let s = setup(Backend::Auto);
    s.native
        .script()
        .describe
        .push_err(OxiError::forbidden("no"));
    let error = s.describer.describe(&pod_ref()).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert!(s.kubectl.recorded_calls().is_empty());
}

#[tokio::test]
async fn auto_without_kubectl_explains_both() {
    let s = setup(Backend::Auto);
    s.native.script().describe.push_err(OxiError::unsupported(
        "native describe does not cover Gadget",
    ));
    s.kubectl
        .script()
        .describe
        .push_err(OxiError::unsupported("kubectl was not found"));
    let error = s.describer.describe(&pod_ref()).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(error.message().contains("native describe"), "{error}");
    assert!(error.message().contains("kubectl was not found"), "{error}");
}

#[tokio::test]
async fn the_setting_picks_the_backend_at_each_call() {
    let s = setup(Backend::Native);
    s.native
        .script()
        .describe
        .push_ok(out("n", DescribeSource::Native));
    assert_eq!(s.describer.describe(&pod_ref()).await.unwrap().text, "n");
    assert!(s.kubectl.recorded_calls().is_empty());

    // The setting changes while the describer lives (a hot reload).
    s.preference.set(DescribeConfig {
        backend: Backend::Kubectl,
        kubectl_path: None,
    });
    s.kubectl
        .script()
        .describe
        .push_ok(out("k", DescribeSource::KubectlFallback));
    assert_eq!(s.describer.describe(&pod_ref()).await.unwrap().text, "k");
    assert_eq!(
        s.native.recorded_calls().len(),
        1,
        "native was not asked again"
    );
}

#[tokio::test]
async fn native_only_reports_an_uncovered_kind_instead_of_falling_back() {
    let s = setup(Backend::Native);
    s.native.script().describe.push_err(OxiError::unsupported(
        "native describe does not cover Gadget",
    ));
    let error = s.describer.describe(&pod_ref()).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert!(s.kubectl.recorded_calls().is_empty());
}
