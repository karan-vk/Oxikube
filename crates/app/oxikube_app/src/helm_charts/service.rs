//! [`ChartsService`]: readiness, cached search and cached chart details.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use oxikube_domain::helm::{ChartPart, ChartRef, ChartSummary, HelmCapabilities, RepoEntry};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::HelmPort;
use parking_lot::Mutex;
use tokio::sync::watch;

use super::catalog::{self, Catalog, Loading};
use crate::helm_repos::RepoChanges;

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
    /// Every chart of every repository (`helm search repo --versions`); a keyword filters it in
    /// memory, so typing in the search field never starts a `helm` process.
    catalog: Option<Arc<Catalog>>,
    /// The catalog's load in flight, shared by every search waiting for it.
    loading: Option<Loading>,
    /// The configured repository aliases: a chart is shown only from one of them.
    repos: Option<Arc<Vec<String>>>,
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
                    catalog: None,
                    loading: None,
                    repos: None,
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
        let generation = self.fresh().generation;
        let listed = self.inner.helm.repos().await;
        if !self.capabilities().can_search_repos {
            return Ok(Readiness::NoHelm);
        }
        let repos = listed?;
        self.remember_repos(generation, &repos);
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
            caches.catalog = None;
            caches.loading = None;
            caches.repos = None;
            caches.details.clear();
        }
        caches
    }

    /// The charts matching `keyword` (every chart when empty), each with all its versions, newest
    /// first. The whole catalog is read with one `helm search repo --versions` per catalog
    /// generation (until a repository changes) and every keyword filters it in memory: a match is
    /// a case-insensitive substring of `repo/name` or of a version's description, like Helm's own
    /// literal search.
    ///
    /// # Errors
    ///
    /// [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) without a usable `helm`; otherwise
    /// the adapter's redacted failure.
    pub async fn search(&self, keyword: &str) -> OxiResult<Arc<Vec<ChartSummary>>> {
        let catalog = self.catalog().await?;
        let key = keyword.trim().to_lowercase();
        Ok(if key.is_empty() {
            catalog.all()
        } else {
            catalog.matching(&key)
        })
    }

    /// Every chart, from the cache or the one load in flight (started here when there is none).
    async fn catalog(&self) -> OxiResult<Arc<Catalog>> {
        let (generation, loading) = {
            let mut caches = self.fresh();
            if let Some(hit) = &caches.catalog {
                return Ok(hit.clone());
            }
            let helm = self.inner.helm.clone();
            let loading = caches
                .loading
                .get_or_insert_with(|| catalog::load(helm))
                .clone();
            (caches.generation, loading)
        };
        let loaded = loading.await;
        let mut caches = self.inner.caches.lock();
        if caches.generation == generation {
            // A failure is not kept: the next search tries again.
            caches.loading = None;
            if let Ok(catalog) = &loaded {
                caches.catalog = Some(catalog.clone());
            }
        }
        loaded.map_err(OxiError::from)
    }

    /// Keeps `repos` as the configured aliases when no repository changed since `generation`
    /// (when the list was asked for).
    fn remember_repos(&self, generation: u64, repos: &[RepoEntry]) {
        let mut caches = self.fresh();
        if caches.generation == generation {
            caches.repos = Some(Arc::new(repos.iter().map(|r| r.name.clone()).collect()));
        }
    }

    /// Whether `repo` is a configured repository alias: from the cache, else (also when the
    /// cache does not have it: it may have been added outside the app) `helm repo list`.
    async fn is_configured(&self, repo: &str) -> OxiResult<bool> {
        let (generation, cached) = {
            let caches = self.fresh();
            (caches.generation, caches.repos.clone())
        };
        if cached.is_some_and(|names| names.iter().any(|name| name == repo)) {
            return Ok(true);
        }
        let repos = self.inner.helm.repos().await?;
        self.remember_repos(generation, &repos);
        Ok(repos.iter().any(|r| r.name == repo))
    }

    /// `part` of `chart` (the newest version when `chart.version` is empty). Cached per chart
    /// version and part until a repository changes. The text can be large (READMEs) and is shared.
    ///
    /// The chart must come from a configured repository and its name must be a plain chart name:
    /// `helm show` also takes URLs and local paths, and `helm::ShowChart` is a read an agent may
    /// run without asking, so it must never fetch from an arbitrary host or read the disk.
    ///
    /// # Errors
    ///
    /// A [`Validation`](oxikube_domain::ErrorKind::Validation) error for a chart outside the
    /// configured repositories or with a name that is not a chart name; otherwise as
    /// [`search`](Self::search), and [`NotFound`](oxikube_domain::ErrorKind::NotFound)-like
    /// failures when the chart or version is gone from the repository.
    pub async fn detail(&self, chart: &ChartRef, part: ChartPart) -> OxiResult<Arc<str>> {
        let Some(repo) = &chart.repo else {
            return Err(OxiError::validation(
                "a chart needs its repository to be shown",
            ));
        };
        if !is_chart_name(&chart.name) {
            return Err(OxiError::validation(format!(
                "{:?} is not a chart name (letters, digits, '-', '_' and '.')",
                chart.name
            )));
        }
        if !self.is_configured(repo).await? {
            return Err(OxiError::validation(format!(
                "{repo:?} is not a configured Helm repository"
            )));
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

/// A chart name as repositories publish them: no path, URL or flag.
fn is_chart_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(['.', '-'])
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}
