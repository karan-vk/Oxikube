//! Tests of the repositories service over the testkit's `FakeHelmPort`.

mod commands;
mod errors;
mod service;

use std::sync::Arc;

use oxikube_domain::helm::RepoEntry;
use oxikube_testkit::{FakeClockPort, FakeHelmPort};

use super::HelmReposService;

pub(super) fn entry(name: &str) -> RepoEntry {
    RepoEntry {
        name: name.into(),
        url: format!("https://{name}.example.com"),
    }
}

pub(super) fn service(fake: &Arc<FakeHelmPort>) -> HelmReposService {
    HelmReposService::new(
        fake.clone(),
        fake.clone(),
        Arc::new(FakeClockPort::default()),
    )
}
