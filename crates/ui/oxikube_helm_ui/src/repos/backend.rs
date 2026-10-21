//! [`ReposBackend`]: where the page reads its state from.

use futures::StreamExt as _;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use jiff::Timestamp;
use oxikube_app::{HelmReposService, RepoSnapshot};
use oxikube_domain::helm::RepoPassword;

/// The page's view of the app. Reads are synchronous and in memory; changes arrive as a stream;
/// work that touches Helm is a future the page runs on the runtime, never on the UI thread.
/// Commands do not go through here: the page sends them on the bus like every other view.
pub trait ReposBackend: 'static {
    /// The current state. Never waits.
    fn snapshot(&self) -> RepoSnapshot;

    /// The current time, for "updated 5 min ago".
    fn now(&self) -> Timestamp;

    /// One item whenever [`snapshot`](Self::snapshot) may have a new answer.
    fn changes(&self) -> BoxStream<'static, ()>;

    /// Reads Helm's repository list. Run it with `spawn_kube`.
    fn refresh(&self) -> BoxFuture<'static, ()>;

    /// Keeps `password` in memory for the next `helm::RepoAdd` of `name`.
    fn stage_password(&self, name: &str, password: RepoPassword);

    /// Forgets a staged password.
    fn discard_password(&self, name: &str);
}

/// The backend over the app's [`HelmReposService`].
#[derive(Clone)]
pub struct ServiceBackend {
    service: HelmReposService,
}

impl ServiceBackend {
    /// A backend over `service`.
    pub fn new(service: HelmReposService) -> Self {
        Self { service }
    }
}

impl ReposBackend for ServiceBackend {
    fn snapshot(&self) -> RepoSnapshot {
        self.service.snapshot()
    }

    fn now(&self) -> Timestamp {
        self.service.now()
    }

    fn changes(&self) -> BoxStream<'static, ()> {
        let changes = self.service.changes();
        futures::stream::unfold(changes, |mut changes| async move {
            changes.changed().await.then_some(((), changes))
        })
        .boxed()
    }

    fn refresh(&self) -> BoxFuture<'static, ()> {
        let service = self.service.clone();
        Box::pin(async move {
            // The outcome is in the snapshot's `problem`.
            service.refresh().await.ok();
        })
    }

    fn stage_password(&self, name: &str, password: RepoPassword) {
        self.service.stage_password(name, password);
    }

    fn discard_password(&self, name: &str) {
        self.service.discard_password(name);
    }
}
