//! Tests of the sources service over `oxikube_testkit` fakes: a fake cluster source, an
//! in-memory filesystem and an in-memory list. No runtime, no threads: the fakes answer at once.

mod add;
mod commands;
mod names;
mod paste;
mod remove;
mod rows;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use futures::executor::block_on;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::UserSource;
use oxikube_testkit::{FakeClusterSourcePort, FakeFsPort};

use super::{KubeconfigSourcesService, MemorySourceList, SourceListStore};

/// Where the test config dir keeps pasted kubeconfigs.
pub(super) const DIR: &str = "/config/kubeconfigs";

/// A kubeconfig with `n` contexts the fake validator accepts.
pub(super) fn kubeconfig(n: usize) -> String {
    let mut text = String::from("apiVersion: v1\nkind: Config\ncontexts:\n");
    for i in 0..n {
        text.push_str(&format!("- context:\n    cluster: c{i}\n  name: c{i}\n"));
    }
    text
}

/// The service and the fakes behind it.
pub(super) struct Fixture {
    pub source: Arc<FakeClusterSourcePort>,
    pub fs: Arc<FakeFsPort>,
    pub list: Arc<MemorySourceList>,
    pub service: KubeconfigSourcesService,
}

impl Fixture {
    /// A fixture whose stored list is `list`.
    pub fn new(list: impl IntoIterator<Item = UserSource>) -> Self {
        let source = Arc::new(FakeClusterSourcePort::new());
        let fs = Arc::new(FakeFsPort::new());
        let list = Arc::new(MemorySourceList::new(list));
        let service = KubeconfigSourcesService::new(
            source.clone(),
            fs.clone(),
            list.clone(),
            PathBuf::from(DIR),
        );
        Self {
            source,
            fs,
            list,
            service,
        }
    }

    /// A fixture with the default entry only, as `default.json` ships.
    pub fn with_defaults() -> Self {
        Self::new([UserSource::default_source()])
    }

    /// Runs `future` to completion.
    pub fn run<T>(&self, future: impl std::future::Future<Output = T>) -> T {
        block_on(future)
    }
}

pub(super) fn file(path: &str) -> UserSource {
    UserSource::file(path)
}

pub(super) fn dir(path: &str) -> UserSource {
    UserSource::dir(path)
}

pub(super) fn stored(name: &str) -> PathBuf {
    Path::new(DIR).join(name)
}

/// A list store that cannot save.
pub(super) struct ReadOnlyList(pub Vec<UserSource>);

#[async_trait]
impl SourceListStore for ReadOnlyList {
    async fn load(&self) -> OxiResult<Vec<UserSource>> {
        Ok(self.0.clone())
    }

    async fn save(&self, _: &[UserSource]) -> OxiResult<()> {
        Err(OxiError::internal("settings.json cannot be written"))
    }
}
