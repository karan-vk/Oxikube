//! A backend with canned releases and no cluster behind it, for the `--perf` scenario and anyone
//! who wants to look at the page: [`StaticBackend`] lists [`fixture`]'s releases at once and
//! answers every detail read with a small fixed text.

use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::LocalBoxStream;
use jiff::Timestamp;
use oxikube_app::helm_releases::{ManifestView, ValuesView};
use oxikube_domain::OxiResult;
use oxikube_domain::helm::{ChartRef, Release, ReleaseRef, ReleaseStatus, Revision};
use oxikube_domain::ids::{ClusterId, ContextName};
use tokio::sync::watch;

use super::backend::{ObjectRow, ObjectsLease, ReleasesBackend};

/// The cluster the fixture's releases are in.
pub fn cluster() -> ClusterId {
    ClusterId::new("preview", &ContextName::from("preview"))
}

/// `count` releases across 12 namespaces and the nine statuses (mostly `deployed`), the way a busy
/// cluster looks: names, charts and versions vary, so sorting and filtering have something to do.
pub fn fixture(count: usize) -> Vec<Release> {
    const CHARTS: [&str; 8] = [
        "nginx",
        "postgresql",
        "redis",
        "prometheus",
        "grafana",
        "cert-manager",
        "ingress-nginx",
        "kafka",
    ];
    let cluster = cluster();
    (0..count)
        .map(|i| {
            let status = match i % 23 {
                0 => ReleaseStatus::Failed,
                1 => ReleaseStatus::PendingUpgrade,
                2 => ReleaseStatus::Superseded,
                3 => ReleaseStatus::Uninstalled,
                _ => ReleaseStatus::Deployed,
            };
            let chart = CHARTS[i % CHARTS.len()];
            let mut release = Release::summary(
                ReleaseRef::new(
                    cluster.clone(),
                    format!("team-{}", i % 12),
                    format!("{chart}-{i}"),
                ),
                ChartRef::new(chart, format!("{}.{}.{}", 1 + i % 15, i % 9, i % 4)),
                1 + (i % 17) as u32,
                status,
                Timestamp::from_second(1_791_806_400 - (i as i64) * 3_600).expect("in range"),
            );
            release.app_version = Some(format!("{}.{}.0", 1 + i % 4, i % 20));
            release
        })
        .collect()
}

/// A backend that lists `count` [`fixture`] releases at once and nothing changes. Every release
/// answers its detail reads with the same canned texts; change them with the `with_*` builders.
pub struct StaticBackend {
    cluster: ClusterId,
    releases: Vec<Release>,
    reveal: watch::Sender<u64>,
    values: Arc<str>,
    sensitive: usize,
    manifest: Arc<str>,
    notes: Option<Arc<str>>,
    objects: Vec<ObjectRow>,
    history: Vec<Revision>,
}

impl StaticBackend {
    /// A backend over `count` releases.
    pub fn new(count: usize) -> Self {
        Self {
            cluster: cluster(),
            releases: fixture(count),
            reveal: watch::channel(0).0,
            values: Arc::from(VALUES),
            sensitive: 0,
            manifest: Arc::from("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: cfg\n"),
            notes: None,
            objects: Vec::new(),
            history: Vec::new(),
        }
    }

    /// The Values tab's text, and how many values it says are masked.
    #[must_use]
    pub fn with_values(mut self, text: &str, masked: usize) -> Self {
        self.values = Arc::from(text);
        self.sensitive = masked;
        self
    }

    /// The Manifest tab's text.
    #[must_use]
    pub fn with_manifest(mut self, text: &str) -> Self {
        self.manifest = Arc::from(text);
        self
    }

    /// The Notes tab's text.
    #[must_use]
    pub fn with_notes(mut self, text: &str) -> Self {
        self.notes = Some(Arc::from(text));
        self
    }

    /// The objects the Resources tab shows.
    #[must_use]
    pub fn with_objects(mut self, objects: Vec<ObjectRow>) -> Self {
        self.objects = objects;
        self
    }

    /// The revisions the History tab shows.
    #[must_use]
    pub fn with_history(mut self, history: Vec<Revision>) -> Self {
        self.history = history;
        self
    }
}

struct FixedObjects(Vec<ObjectRow>);

impl ObjectsLease for FixedObjects {
    fn objects(&self) -> Vec<ObjectRow> {
        self.0.clone()
    }
}

const VALUES: &str = "replicaCount: 2\nimage:\n  repository: nginx\n  tag: \"1.25\"\n";

impl ReleasesBackend for StaticBackend {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    fn list(&self) -> BoxFuture<'static, OxiResult<Vec<Release>>> {
        let releases = self.releases.clone();
        Box::pin(async move { Ok(releases) })
    }

    fn watch(&self) -> Option<LocalBoxStream<'static, ()>> {
        None
    }

    fn history(&self, _: &ReleaseRef) -> BoxFuture<'static, OxiResult<Vec<Revision>>> {
        let history = self.history.clone();
        Box::pin(async move { Ok(history) })
    }

    fn values(&self, _: &ReleaseRef, _: u32, _: bool) -> BoxFuture<'static, OxiResult<ValuesView>> {
        let (text, sensitive) = (self.values.clone(), self.sensitive);
        Box::pin(async move {
            Ok(ValuesView {
                text,
                sensitive,
                revealed: false,
                empty: false,
            })
        })
    }

    fn manifest(&self, _: &ReleaseRef, _: u32) -> BoxFuture<'static, OxiResult<ManifestView>> {
        let text = self.manifest.clone();
        Box::pin(async move { Ok(ManifestView { text, masked: true }) })
    }

    fn notes(&self, _: &ReleaseRef, _: u32) -> BoxFuture<'static, OxiResult<Option<Arc<str>>>> {
        let notes = self.notes.clone();
        Box::pin(async move { Ok(notes) })
    }

    fn objects(
        &self,
        _: &ReleaseRef,
        _: u32,
        _: Timestamp,
    ) -> BoxFuture<'static, OxiResult<Box<dyn ObjectsLease>>> {
        let objects = self.objects.clone();
        Box::pin(async move { Ok(Box::new(FixedObjects(objects)) as Box<dyn ObjectsLease>) })
    }

    fn reveal_changes(&self) -> watch::Receiver<u64> {
        self.reveal.subscribe()
    }

    fn is_revealed(&self, _: &ReleaseRef) -> bool {
        false
    }

    fn conceal(&self, _: &ReleaseRef) {}
}
