//! `HelmService`: parsed once per release revision, errors from the port pass through.

use std::sync::Arc;

use oxikube_testkit::{FakeHelmPort, HelmCall};

use super::{release, served};
use crate::helm::HelmService;

fn manifest_calls(fake: &FakeHelmPort) -> usize {
    fake.recorded_calls()
        .iter()
        .filter(|c| matches!(c, HelmCall::Manifest { .. }))
        .count()
}

#[tokio::test]
async fn a_revision_is_read_and_parsed_once() {
    let fake = Arc::new(FakeHelmPort::new());
    fake.script().manifest.push_ok(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cfg\n---\nkey: [broken\n".into(),
    );
    let service = HelmService::new(fake.clone());
    let plan = service.plan(&release(), 3, &served()).await.unwrap();
    assert_eq!(plan.resources.len(), 1);
    assert_eq!(
        plan.errors.len(),
        1,
        "one bad document does not drop the rest"
    );
    // A second ask (nothing else is scripted) is served from the cache.
    let again = service.plan(&release(), 3, &served()).await.unwrap();
    assert_eq!(again.resources.len(), 1);
    assert_eq!(manifest_calls(&fake), 1);
}

#[tokio::test]
async fn a_port_error_is_returned() {
    let fake = Arc::new(FakeHelmPort::new());
    fake.script()
        .manifest
        .push_err(oxikube_domain::OxiError::internal("boom"));
    let service = HelmService::new(fake);
    assert!(service.plan(&release(), 1, &served()).await.is_err());
}
