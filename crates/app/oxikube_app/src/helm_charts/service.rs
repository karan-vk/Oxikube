//! [`ChartsService`]: readiness, cached search and cached chart details.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use oxikube_domain::helm::{
    ChartPart, ChartRef, ChartSummary, HelmCapabilities, RepoEntry, SearchQuery, group_hits,
};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::HelmPort;
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::helm_repos::RepoChanges;

/// How many distinct keywords' results are kept.
const SEARCHES_KEPT: usize = 8;
/// How many chart details (READMEs, values) are kept.
const DETAILS_KEPT: usize = 48;

/// Whether the catalog can be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// No usable `helm` binary: repositories, search and show all need it.
    NoHelm,
    /// `helm` works but no repository is configured.
    NoRepos,
    /// These repositories are configured.
    Ready(Vec<RepoEntry>),
}

/// A bounded cache that forgets the oldest entry first.
struct Bounded<K, V> {
    map: HashMap<K, V>,
    order: VecDeque<K>,
    cap: usize,
}

impl<K: std::hash::Hash + Eq + Clone, V: Clone> Bounded<K, V> {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    fn get(&self, key: &K) -> Option<V> {
        self.map.get(key).cloned()
    }

    fn put(&mut self, key: K, value: V) {
        if self.map.insert(key.clone(), value).is_none() {
            self.order.push_back(key);
            while self.order.len() > self.cap {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
    }

    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
}

struct Caches {
    /// The catalog generation these entries belong to.
    generation: u64,
    searches: Bounded<String, Arc<Vec<ChartSummary>>>,
    details: Bounded<(ChartRef, ChartPart), Arc<str>>,
}

struct Inner {
    helm: Arc<dyn HelmPort>,
    catalog: RepoChanges,
    caches: Mutex<Caches>,
    install: watch::Sender<Option<ChartRef>>,
}

/// The charts catalog. Cheap to clone. See the [module docs](super).
#[derive(Clone)]
pub struct ChartsService {
    inner: Arc<Inner>,
}

impl ChartsService {
    /// A catalog over `helm`, dropping its caches when `catalog` says repositories changed
    /// ([`HelmReposService::catalog_changes`](crate::HelmReposService::catalog_changes)).
    pub fn new(helm: Arc<dyn HelmPort>, catalog: RepoChanges) -> Self {
        let generation = catalog.generation();
        Self {
            inner: Arc::new(Inner {
                helm,
                catalog,
                caches: Mutex::new(Caches {
                    generation,
                    searches: Bounded::new(SEARCHES_KEPT),
                    details: Bounded::new(DETAILS_KEPT),
                }),
                install: watch::channel(None).0,
            }),
        }
    }

    /// Wakes when a repository was added, removed or refreshed: the catalog should be read again.
    pub fn catalog_changes(&self) -> RepoChanges {
        self.inner.catalog.clone()
    }

    /// What the adapter can do now (cheap; detection happens on the first [`ready`](Self::ready)).
    pub fn capabilities(&self) -> HelmCapabilities {
        self.inner.helm.capabilities()
    }

    /// Whether the catalog can be used. The first call makes the adapter look for the `helm`
    /// binary, so run it off the UI thread.
    ///
    /// # Errors
    ///
    /// Whatever listing the repositories failed with, other than a missing binary.
    pub async fn ready(&self) -> OxiResult<Readiness> {
        let listed = self.inner.helm.repos().await;
        if !self.capabilities().can_search_repos {
            return Ok(Readiness::NoHelm);
        }
        let repos = listed?;
        Ok(if repos.is_empty() {
            Readiness::NoRepos
        } else {
            Readiness::Ready(repos)
        })
    }

    /// Drops everything cached if repositories changed since it was cached.
    fn fresh(&self) -> parking_lot::MutexGuard<'_, Caches> {
        let mut caches = self.inner.caches.lock();
        let generation = self.inner.catalog.generation();
        if caches.generation != generation {
            caches.generation = generation;
            caches.searches.clear();
            caches.details.clear();
        }
        caches
    }

    /// The charts matching `keyword` (every chart when empty), each with all its versions, newest
    /// first. Cached per keyword until a repository changes.
    ///
    /// # Errors
    ///
    /// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) without a usable `helm`; otherwise
    /// the adapter's redacted failure.
    pub async fn search(&self, keyword: &str) -> OxiResult<Arc<Vec<ChartSummary>>> {
        let key = keyword.trim().to_lowercase();
        let generation = {
            let caches = self.fresh();
            if let Some(hit) = caches.searches.get(&key) {
                return Ok(hit);
            }
            caches.generation
        };
        let hits = self
            .inner
            .helm
            .search(&SearchQuery {
                keyword: keyword.trim().to_owned(),
                all_versions: true,
            })
            .await?;
        let charts = Arc::new(group_hits(hits));
        let mut caches = self.inner.caches.lock();
        if caches.generation == generation {
            caches.searches.put(key, charts.clone());
        }
        Ok(charts)
    }

    /// `part` of `chart` (the newest version when `chart.version` is empty). Cached per chart
    /// version and part until a repository changes. The text can be large (READMEs) and is shared.
    ///
    /// # Errors
    ///
    /// As [`search`](Self::search); [`NotFound`](oxikube_domain::ErrorKind::NotFound)-like
    /// failures when the chart or version is gone from the repository.
    pub async fn detail(&self, chart: &ChartRef, part: ChartPart) -> OxiResult<Arc<str>> {
        if chart.repo.is_none() {
            return Err(OxiError::validation(
                "a chart needs its repository to be shown",
            ));
        }
        let key = (chart.clone(), part);
        let generation = {
            let caches = self.fresh();
            if let Some(hit) = caches.details.get(&key) {
                return Ok(hit);
            }
            caches.generation
        };
        let text: Arc<str> = self.inner.helm.show(chart, part.show_kind()).await?.into();
        let mut caches = self.inner.caches.lock();
        if caches.generation == generation {
            caches.details.put(key, text.clone());
        }
        Ok(text)
    }

    /// Hands `chart` to the install tab (E16-S09): it follows
    /// [`install_requests`](Self::install_requests). Nothing is installed.
    pub fn request_install(&self, chart: ChartRef) {
        self.inner.install.send_replace(Some(chart));
    }

    /// The chart the last `helm::InstallChart` asked to install, for the install tab to prefill
    /// from; `None` until one was asked.
    pub fn install_requests(&self) -> watch::Receiver<Option<ChartRef>> {
        self.inner.install.subscribe()
    }
}
