//! [`HelmReposService`]: the queue, the state and the flows.

use std::collections::HashMap;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::helm::{RepoAddRequest, RepoEntry, RepoPassword};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ClockPort, HelmPort, HelmRepoPort};
use parking_lot::Mutex;
use tokio::sync::{Mutex as AsyncMutex, watch};

use super::errors::describe_error;
use super::model::{RepoActivity, RepoRow, RepoSnapshot, RepoStatus, UpdateReport};

/// How long a staged password waits for its `helm::RepoAdd` before it is forgotten, in seconds.
const STAGED_TTL_SECS: i64 = 120;

struct Inner {
    helm: Arc<dyn HelmPort>,
    repos: Arc<dyn HelmRepoPort>,
    clock: Arc<dyn ClockPort>,
    /// Held for the whole of every operation: our own repository changes run one at a time.
    queue: AsyncMutex<()>,
    state: Mutex<RepoSnapshot>,
    /// Passwords typed in the add dialog, waiting for their `helm::RepoAdd`. Memory only.
    passwords: Mutex<HashMap<String, (RepoPassword, Timestamp)>>,
    /// Bumped whenever [`state`](Self::state) changes.
    changes: watch::Sender<u64>,
    /// Bumped whenever the set of repositories or their indexes changed (the catalog's cue).
    catalog: watch::Sender<u64>,
}

/// The repositories in Helm's own configuration. Cheap to clone. See the [module docs](super).
#[derive(Clone)]
pub struct HelmReposService {
    inner: Arc<Inner>,
}

/// Wakes when the service's [`RepoSnapshot`] changed (or, from
/// [`catalog_changes`](HelmReposService::catalog_changes), when cached searches went stale).
#[derive(Clone)]
pub struct RepoChanges(watch::Receiver<u64>);

impl RepoChanges {
    /// Waits for the next change; `false` once the service is gone.
    pub async fn changed(&mut self) -> bool {
        self.0.changed().await.is_ok()
    }

    /// `true` when a change arrived that [`changed`](Self::changed) has not yet returned.
    pub fn has_changed(&self) -> bool {
        self.0.has_changed().unwrap_or(false)
    }

    /// How many catalog invalidations there have been (a cheap cache key).
    pub fn generation(&self) -> u64 {
        *self.0.borrow()
    }
}

impl HelmReposService {
    /// A service listing through `helm` and changing repositories through `repos`.
    pub fn new(
        helm: Arc<dyn HelmPort>,
        repos: Arc<dyn HelmRepoPort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        let cli_available = helm.capabilities().cli_available;
        Self {
            inner: Arc::new(Inner {
                helm,
                repos,
                clock,
                queue: AsyncMutex::new(()),
                state: Mutex::new(RepoSnapshot {
                    cli_available,
                    ..RepoSnapshot::default()
                }),
                passwords: Mutex::new(HashMap::new()),
                changes: watch::channel(0).0,
                catalog: watch::channel(0).0,
            }),
        }
    }

    /// The service's clock: "updated 5 min ago" is relative to it.
    pub fn now(&self) -> jiff::Timestamp {
        self.inner.clock.now()
    }

    /// What the page shows now. In memory; never waits.
    pub fn snapshot(&self) -> RepoSnapshot {
        let mut snapshot = self.inner.state.lock().clone();
        snapshot.cli_available = self.inner.helm.capabilities().cli_available;
        snapshot
    }

    /// Follows [`snapshot`](Self::snapshot) changes.
    pub fn changes(&self) -> RepoChanges {
        RepoChanges(self.inner.changes.subscribe())
    }

    /// Follows the cue for the charts catalog to drop cached searches: a repository was added,
    /// removed or refreshed.
    pub fn catalog_changes(&self) -> RepoChanges {
        RepoChanges(self.inner.catalog.subscribe())
    }

    /// Keeps `password` in memory for the next `helm::RepoAdd` of repository `name`, for two
    /// minutes at most. Older staged passwords are forgotten here, so one whose command never ran
    /// (the bus refused it) does not wait around for somebody else's add.
    pub fn stage_password(&self, name: &str, password: RepoPassword) {
        let now = self.inner.clock.now();
        let mut staged = self.inner.passwords.lock();
        staged.retain(|_, (_, at)| now.as_second() - at.as_second() <= STAGED_TTL_SECS);
        staged.insert(name.to_owned(), (password, now));
    }

    /// Takes the password staged for `name`, if it is still fresh.
    fn take_password(&self, name: &str) -> Option<RepoPassword> {
        let (password, at) = self.inner.passwords.lock().remove(name)?;
        let age = self.inner.clock.now().as_second() - at.as_second();
        (age <= STAGED_TTL_SECS).then_some(password)
    }

    /// Forgets a staged password (the dialog was closed).
    pub fn discard_password(&self, name: &str) {
        self.inner.passwords.lock().remove(name);
    }

    /// Reads Helm's list. Statuses of repositories still listed are kept.
    ///
    /// # Errors
    ///
    /// The port's error (no binary, `helm` failed); it is also in the snapshot's `problem`.
    pub async fn refresh(&self) -> OxiResult<()> {
        let _turn = self.inner.queue.lock().await;
        self.begin(RepoActivity::Loading);
        let result = self.read_list().await;
        self.mutate(|s| s.checked = true);
        self.finish(result.as_ref().err(), None);
        result.map(drop)
    }

    /// Adds a repository (`helm repo add`) and lists again. A password staged for the name is
    /// used and forgotten.
    ///
    /// # Errors
    ///
    /// A validation error for a bad name or URL, else the port's error.
    pub async fn add(&self, request: RepoAddRequest) -> OxiResult<()> {
        self.add_with(request, true).await
    }

    /// [`add`](Self::add), choosing whether a staged password may be used. An agent's add never
    /// uses one: that password was typed by the user for the user's own add.
    ///
    /// # Errors
    ///
    /// As [`add`](Self::add).
    pub async fn add_with(&self, mut request: RepoAddRequest, use_staged: bool) -> OxiResult<()> {
        request.name = request.name.trim().to_owned();
        request.url = request.url.trim().to_owned();
        oxikube_domain::helm::validate_repo(&request.name, &request.url)
            .map_err(OxiError::validation)?;
        if use_staged && let Some(password) = self.take_password(&request.name) {
            request.password = Some(password);
        }
        let _turn = self.inner.queue.lock().await;
        self.begin(RepoActivity::Adding(request.name.clone()));
        let added = self.inner.repos.repo_add(&request).await;
        let mut relist_problem = None;
        if added.is_ok() {
            if let Err(error) = self.read_list().await {
                relist_problem = Some(format!(
                    "Added {}, but reading Helm's list again failed: {}",
                    request.name,
                    describe_error(&error)
                ));
            }
            // `repo add` fetched the index, so the repository is fresh.
            self.set_status(&request.name, RepoStatus::Updated(self.inner.clock.now()));
        }
        let notice = format!("Added repository {}", request.name);
        match relist_problem {
            Some(problem) => self.end(Some(problem), Some(notice)),
            None => self.finish(added.as_ref().err(), added.is_ok().then_some(notice)),
        }
        self.invalidate_catalog_if(added.is_ok());
        added
    }

    /// Removes a repository (`helm repo remove`) and lists again.
    ///
    /// # Errors
    ///
    /// The port's error.
    pub async fn remove(&self, name: &str) -> OxiResult<()> {
        let _turn = self.inner.queue.lock().await;
        self.begin(RepoActivity::Removing(name.to_owned()));
        let removed = self.inner.repos.repo_remove(name).await;
        let mut relist_problem = None;
        if removed.is_ok()
            && let Err(error) = self.read_list().await
        {
            relist_problem = Some(format!(
                "Removed {name}, but reading Helm's list again failed: {}",
                describe_error(&error)
            ));
        }
        let notice = format!("Removed repository {name}");
        match relist_problem {
            Some(problem) => self.end(Some(problem), Some(notice)),
            None => self.finish(removed.as_ref().err(), removed.is_ok().then_some(notice)),
        }
        self.invalidate_catalog_if(removed.is_ok());
        removed
    }

    /// Refreshes `names` (every repository when empty), one at a time so each gets its own
    /// result. Failures do not stop the rest.
    ///
    /// # Errors
    ///
    /// Only for what stops the whole run (no binary). Per-repository failures are in the report
    /// and in the rows' statuses.
    pub async fn update(&self, names: &[String]) -> OxiResult<UpdateReport> {
        let _turn = self.inner.queue.lock().await;
        let targets: Vec<String> = if names.is_empty() {
            // "Every repository" is what Helm lists now, not what the page cached earlier.
            self.begin(RepoActivity::Loading);
            if let Err(error) = self.read_list().await {
                self.mutate(|s| s.checked = true);
                self.finish(Some(&error), None);
                return Err(error);
            }
            self.inner
                .state
                .lock()
                .rows
                .iter()
                .map(|r| r.entry.name.clone())
                .collect()
        } else {
            names.to_vec()
        };
        self.begin(RepoActivity::Updating(targets.clone()));
        for name in &targets {
            self.set_status(name, RepoStatus::Updating);
        }
        let mut report = UpdateReport::default();
        let mut fatal = None;
        for name in &targets {
            match self
                .inner
                .repos
                .repo_update(std::slice::from_ref(name))
                .await
            {
                Ok(()) => {
                    self.set_status(name, RepoStatus::Updated(self.inner.clock.now()));
                    report.updated.push(name.clone());
                }
                Err(error) if error.kind() == oxikube_domain::ErrorKind::Unsupported => {
                    fatal = Some(error);
                    break;
                }
                Err(error) => {
                    let why = describe_error(&error);
                    self.set_status(name, RepoStatus::Failed(why.clone()));
                    report.failed.push((name.clone(), why));
                }
            }
        }
        if let Some(error) = fatal {
            // What finished keeps its status; what never ran goes back to unknown.
            for name in &targets {
                let done =
                    report.updated.contains(name) || report.failed.iter().any(|(n, _)| n == name);
                if !done {
                    self.set_status(name, RepoStatus::Unknown);
                }
            }
            self.finish(Some(&error), None);
            self.invalidate_catalog_if(!report.updated.is_empty());
            return Err(error);
        }
        let notice = report.summary();
        let failed = !report.failed.is_empty();
        self.end(failed.then(|| notice.clone()), (!failed).then_some(notice));
        self.invalidate_catalog_if(!report.updated.is_empty());
        Ok(report)
    }

    /// Lists through the port and merges the result into the state.
    async fn read_list(&self) -> OxiResult<Vec<RepoEntry>> {
        let entries = self.inner.helm.repos().await?;
        let mut state = self.inner.state.lock();
        let old = std::mem::take(&mut state.rows);
        state.rows = entries
            .iter()
            .map(|entry| RepoRow {
                status: old
                    .iter()
                    .find(|r| r.entry.name == entry.name)
                    .map(|r| r.status.clone())
                    .unwrap_or_default(),
                entry: entry.clone(),
            })
            .collect();
        state.loaded = true;
        drop(state);
        Ok(entries)
    }

    fn begin(&self, activity: RepoActivity) {
        self.mutate(|s| {
            s.activity = activity;
            s.problem = None;
            s.notice = None;
        });
    }

    /// Ends an operation: idle again, with `error` shown as the problem.
    fn finish(&self, error: Option<&OxiError>, notice: Option<String>) {
        self.end(error.map(describe_error), notice);
    }

    fn end(&self, problem: Option<String>, notice: Option<String>) {
        self.mutate(|s| {
            s.activity = RepoActivity::Idle;
            s.problem = problem;
            s.notice = notice;
        });
    }

    fn set_status(&self, name: &str, status: RepoStatus) {
        self.mutate(|s| {
            if let Some(row) = s.rows.iter_mut().find(|r| r.entry.name == name) {
                row.status = status;
            }
        });
    }

    fn mutate(&self, edit: impl FnOnce(&mut RepoSnapshot)) {
        edit(&mut self.inner.state.lock());
        self.inner.changes.send_modify(|n| *n += 1);
    }

    fn invalidate_catalog_if(&self, changed: bool) {
        if changed {
            self.inner.catalog.send_modify(|n| *n += 1);
        }
    }
}
