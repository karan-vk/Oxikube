//! [`FakeHelmPort`]: scripted Helm, with configurable capabilities (including native-only).

use async_trait::async_trait;
use futures::StreamExt as _;
use oxikube_domain::helm::HelmError;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    ChartHit, ChartRef, HelmCapabilities, HelmOutput, HelmPort, HelmRepoPort, HelmVersion,
    InstallRequest, OutputLine, Release, ReleaseRef, RepoAddRequest, RepoEntry, Revision,
    RollbackRequest, SearchQuery, ShowKind, UninstallRequest, UpgradeRequest, Values,
};
use parking_lot::Mutex;

use crate::script::{CallLog, Script};

/// Queued responses for each [`FakeHelmPort`] method. `install` and `upgrade` are scripted
/// with the output lines of the stream they return.
#[derive(Debug, Default)]
pub struct HelmScripts {
    /// `list_releases`.
    pub list_releases: Script<Vec<Release>>,
    /// `history`.
    pub history: Script<Vec<Revision>>,
    /// `values`.
    pub values: Script<Values>,
    /// `manifest`.
    pub manifest: Script<String>,
    /// `notes`.
    pub notes: Script<Option<String>>,
    /// `repos`.
    pub repos: Script<Vec<RepoEntry>>,
    /// `search`.
    pub search: Script<Vec<ChartHit>>,
    /// `show`.
    pub show: Script<String>,
    /// `template`.
    pub template: Script<String>,
    /// `install` (mutating): the progress lines.
    pub install: Script<Vec<OutputLine>>,
    /// `upgrade` (mutating): the progress lines.
    pub upgrade: Script<Vec<OutputLine>>,
    /// `rollback` (mutating).
    pub rollback: Script<()>,
    /// `uninstall` (mutating).
    pub uninstall: Script<()>,
    /// `repo_add` (changes local Helm configuration).
    pub repo_add: Script<()>,
    /// `repo_remove` (changes local Helm configuration).
    pub repo_remove: Script<()>,
    /// `repo_update`.
    pub repo_update: Script<()>,
}

/// One call made on a [`FakeHelmPort`]. Calls refused for missing capabilities are
/// recorded too.
#[derive(Debug, Clone, PartialEq)]
pub enum HelmCall {
    /// `list_releases`.
    ListReleases(ClusterId, NamespaceSelection),
    /// `history`.
    History(ReleaseRef),
    /// `values`.
    Values {
        /// Release.
        release: ReleaseRef,
        /// Revision, `None` for the latest.
        revision: Option<u32>,
        /// Computed values instead of user-supplied ones.
        all: bool,
    },
    /// `manifest`.
    Manifest {
        /// Release.
        release: ReleaseRef,
        /// Revision, `None` for the latest.
        revision: Option<u32>,
    },
    /// `notes`.
    Notes {
        /// Release.
        release: ReleaseRef,
        /// Revision, `None` for the latest.
        revision: Option<u32>,
    },
    /// `repos`.
    Repos,
    /// `search`.
    Search(SearchQuery),
    /// `show`.
    Show(ChartRef, ShowKind),
    /// `template` (renders only).
    Template(InstallRequest),
    /// `install` (mutating).
    Install(InstallRequest),
    /// `upgrade` (mutating).
    Upgrade(UpgradeRequest),
    /// `rollback` (mutating).
    Rollback(RollbackRequest),
    /// `uninstall` (mutating).
    Uninstall(UninstallRequest),
    /// `repo_add` (local configuration, not a cluster mutation).
    RepoAdd(RepoAddRequest),
    /// `repo_remove`.
    RepoRemove(String),
    /// `repo_update`: the names, empty for all.
    RepoUpdate(Vec<String>),
}

impl HelmCall {
    /// `true` for `install`, `upgrade`, `rollback` and `uninstall`, the calls
    /// `MutationGuard` must gate. The `Repo*` calls change only local Helm configuration.
    pub fn is_mutating(&self) -> bool {
        matches!(
            self,
            Self::Install(_) | Self::Upgrade(_) | Self::Rollback(_) | Self::Uninstall(_)
        )
    }
}

/// Fake `HelmPort`. Fallback: `list_releases` is empty; everything else is scripted only.
///
/// Capabilities start as [`HelmCapabilities::full`] (a 3.14.0 binary). With
/// [`native_only`](Self::native_only) (or [`set_capabilities`](Self::set_capabilities))
/// every operation whose capability is off fails with `Unsupported`, recorded but without
/// consuming a script, the way a real adapter behaves without a binary.
#[derive(Debug)]
pub struct FakeHelmPort {
    script: HelmScripts,
    calls: CallLog<HelmCall>,
    capabilities: Mutex<HelmCapabilities>,
}

fake_plumbing!(FakeHelmPort, HelmScripts, HelmCall);

impl Default for FakeHelmPort {
    fn default() -> Self {
        Self::with_capabilities(HelmCapabilities::full(HelmVersion::new(3, 14, 0)))
    }
}

impl FakeHelmPort {
    /// A fake with a full-featured binary and nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake with no binary: native reads work, everything else is `Unsupported`.
    pub fn native_only() -> Self {
        Self::with_capabilities(HelmCapabilities::native_only())
    }

    /// A fake reporting `capabilities`.
    pub fn with_capabilities(capabilities: HelmCapabilities) -> Self {
        Self {
            script: HelmScripts::default(),
            calls: CallLog::default(),
            capabilities: Mutex::new(capabilities),
        }
    }

    /// Changes the reported capabilities (a binary appearing or disappearing).
    pub fn set_capabilities(&self, capabilities: HelmCapabilities) {
        *self.capabilities.lock() = capabilities;
    }

    fn check(&self, allowed: impl FnOnce(&HelmCapabilities) -> bool) -> OxiResult<()> {
        if allowed(&self.capabilities.lock()) {
            Ok(())
        } else {
            Err(OxiError::from(HelmError::Unsupported))
        }
    }

    fn can_read(&self) -> OxiResult<()> {
        self.check(|c| c.native_read || c.cli_available)
    }
}

fn lines_stream(lines: OxiResult<Vec<OutputLine>>) -> OxiResult<HelmOutput> {
    lines.map(|lines| futures::stream::iter(lines.into_iter().map(Ok)).boxed())
}

#[async_trait]
impl HelmPort for FakeHelmPort {
    async fn list_releases(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<Vec<Release>> {
        self.calls
            .record(HelmCall::ListReleases(cluster.clone(), namespaces.clone()));
        self.can_read()?;
        self.script.list_releases.next_or_else(|| Ok(Vec::new()))
    }

    async fn history(&self, release: &ReleaseRef) -> OxiResult<Vec<Revision>> {
        self.calls.record(HelmCall::History(release.clone()));
        self.can_read()?;
        self.script
            .history
            .next_or_unscripted("FakeHelmPort", "history")
    }

    async fn values(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
        all: bool,
    ) -> OxiResult<Values> {
        self.calls.record(HelmCall::Values {
            release: release.clone(),
            revision,
            all,
        });
        self.can_read()?;
        self.script
            .values
            .next_or_unscripted("FakeHelmPort", "values")
    }

    async fn manifest(&self, release: &ReleaseRef, revision: Option<u32>) -> OxiResult<String> {
        self.calls.record(HelmCall::Manifest {
            release: release.clone(),
            revision,
        });
        self.can_read()?;
        self.script
            .manifest
            .next_or_unscripted("FakeHelmPort", "manifest")
    }

    async fn notes(
        &self,
        release: &ReleaseRef,
        revision: Option<u32>,
    ) -> OxiResult<Option<String>> {
        self.calls.record(HelmCall::Notes {
            release: release.clone(),
            revision,
        });
        self.can_read()?;
        self.script
            .notes
            .next_or_unscripted("FakeHelmPort", "notes")
    }

    async fn repos(&self) -> OxiResult<Vec<RepoEntry>> {
        self.calls.record(HelmCall::Repos);
        self.check(|c| c.can_search_repos)?;
        self.script
            .repos
            .next_or_unscripted("FakeHelmPort", "repos")
    }

    async fn search(&self, query: &SearchQuery) -> OxiResult<Vec<ChartHit>> {
        self.calls.record(HelmCall::Search(query.clone()));
        self.check(|c| c.can_search_repos)?;
        self.script
            .search
            .next_or_unscripted("FakeHelmPort", "search")
    }

    async fn show(&self, chart: &ChartRef, kind: ShowKind) -> OxiResult<String> {
        self.calls.record(HelmCall::Show(chart.clone(), kind));
        self.check(|c| c.can_search_repos)?;
        self.script.show.next_or_unscripted("FakeHelmPort", "show")
    }

    async fn template(&self, request: &InstallRequest) -> OxiResult<String> {
        self.calls.record(HelmCall::Template(request.clone()));
        self.check(|c| c.can_install)?;
        self.script
            .template
            .next_or_unscripted("FakeHelmPort", "template")
    }

    async fn install(&self, request: &InstallRequest) -> OxiResult<HelmOutput> {
        self.calls.record(HelmCall::Install(request.clone()));
        self.check(|c| c.can_install)?;
        lines_stream(
            self.script
                .install
                .next_or_unscripted("FakeHelmPort", "install"),
        )
    }

    async fn upgrade(&self, request: &UpgradeRequest) -> OxiResult<HelmOutput> {
        self.calls.record(HelmCall::Upgrade(request.clone()));
        self.check(|c| c.can_upgrade)?;
        lines_stream(
            self.script
                .upgrade
                .next_or_unscripted("FakeHelmPort", "upgrade"),
        )
    }

    async fn rollback(&self, request: &RollbackRequest) -> OxiResult<()> {
        self.calls.record(HelmCall::Rollback(request.clone()));
        self.check(|c| c.can_rollback)?;
        self.script
            .rollback
            .next_or_unscripted("FakeHelmPort", "rollback")
    }

    async fn uninstall(&self, request: &UninstallRequest) -> OxiResult<()> {
        self.calls.record(HelmCall::Uninstall(request.clone()));
        self.check(|c| c.can_uninstall)?;
        self.script
            .uninstall
            .next_or_unscripted("FakeHelmPort", "uninstall")
    }

    fn capabilities(&self) -> HelmCapabilities {
        self.capabilities.lock().clone()
    }
}

#[async_trait]
impl HelmRepoPort for FakeHelmPort {
    async fn repo_add(&self, request: &RepoAddRequest) -> OxiResult<()> {
        self.calls.record(HelmCall::RepoAdd(request.clone()));
        self.check(|c| c.cli_available)?;
        self.script.repo_add.next_or_else(|| Ok(()))
    }

    async fn repo_remove(&self, name: &str) -> OxiResult<()> {
        self.calls.record(HelmCall::RepoRemove(name.to_owned()));
        self.check(|c| c.cli_available)?;
        self.script.repo_remove.next_or_else(|| Ok(()))
    }

    async fn repo_update(&self, names: &[String]) -> OxiResult<()> {
        self.calls.record(HelmCall::RepoUpdate(names.to_vec()));
        self.check(|c| c.cli_available)?;
        self.script.repo_update.next_or_else(|| Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt as _;
    use futures::executor::block_on;
    use jiff::Timestamp;
    use oxikube_domain::ErrorKind;
    use oxikube_domain::helm::ReleaseStatus;
    use oxikube_domain::ids::ContextName;

    use super::*;

    fn cluster() -> ClusterId {
        ClusterId::new("kubeconfig-a", &ContextName::new("kind-a"))
    }

    fn release_ref() -> ReleaseRef {
        ReleaseRef::new(cluster(), "demo", "web")
    }

    fn request() -> InstallRequest {
        InstallRequest::new(cluster(), "web", "demo", ChartRef::new("nginx", "1.0.0"))
    }

    fn revision() -> Revision {
        Revision {
            revision: 2,
            status: ReleaseStatus::Deployed,
            chart: ChartRef::new("web", "1.2.3"),
            app_version: Some("1.0".into()),
            updated: Timestamp::UNIX_EPOCH,
            description: "Upgrade complete".into(),
        }
    }

    #[test]
    fn scripted_ok_err_and_calls_recorded() {
        let fake = FakeHelmPort::new();
        let release = release_ref();
        let all = NamespaceSelection::default();
        assert!(
            block_on(fake.list_releases(&cluster(), &all))
                .unwrap()
                .is_empty()
        );
        fake.script().history.push_ok(vec![revision()]);
        assert_eq!(block_on(fake.history(&release)).unwrap(), vec![revision()]);
        fake.script()
            .rollback
            .push_ok(())
            .push_err(OxiError::conflict("in progress"));
        let rollback = RollbackRequest::new(release.clone(), 1);
        block_on(fake.rollback(&rollback)).unwrap();
        assert_eq!(
            block_on(fake.rollback(&rollback)).unwrap_err().kind(),
            ErrorKind::Conflict
        );
        assert!(block_on(fake.values(&release, None, true)).is_err());
        assert!(block_on(fake.manifest(&release, Some(1))).is_err());
        assert!(block_on(fake.uninstall(&UninstallRequest::new(release))).is_err());
        let calls = fake.recorded_calls();
        assert_eq!(calls.len(), 7);
        assert_eq!(calls.iter().filter(|c| c.is_mutating()).count(), 3);
    }

    #[test]
    fn install_streams_scripted_lines_then_errors_end_the_stream() {
        let fake = FakeHelmPort::new();
        fake.script().install.push_ok(vec![
            OutputLine::stdout("NAME: web"),
            OutputLine::stderr("w"),
        ]);
        let lines: Vec<_> =
            block_on(async { fake.install(&request()).await.unwrap().try_collect().await })
                .unwrap();
        assert_eq!(
            lines,
            vec![OutputLine::stdout("NAME: web"), OutputLine::stderr("w")]
        );
        fake.script().upgrade.push_err(OxiError::timeout("5m"));
        let upgrade = UpgradeRequest::new(release_ref(), ChartRef::new("nginx", ""));
        assert_eq!(
            block_on(fake.upgrade(&upgrade)).err().unwrap().kind(),
            ErrorKind::Timeout
        );
    }

    #[test]
    fn native_only_serves_reads_and_refuses_everything_else() {
        let fake = FakeHelmPort::native_only();
        assert!(fake.capabilities().native_read);
        assert!(!fake.capabilities().can_mutate());
        fake.script().manifest.push_ok("kind: Pod".into());
        assert_eq!(
            block_on(fake.manifest(&release_ref(), None)).unwrap(),
            "kind: Pod"
        );
        let unsupported = |kind: ErrorKind| assert_eq!(kind, ErrorKind::Unsupported);
        unsupported(block_on(fake.repos()).unwrap_err().kind());
        unsupported(
            block_on(fake.search(&SearchQuery::default()))
                .unwrap_err()
                .kind(),
        );
        unsupported(
            block_on(fake.show(&ChartRef::new("nginx", ""), ShowKind::Readme))
                .unwrap_err()
                .kind(),
        );
        unsupported(block_on(fake.template(&request())).unwrap_err().kind());
        unsupported(block_on(fake.install(&request())).err().unwrap().kind());
        let upgrade = UpgradeRequest::new(release_ref(), ChartRef::new("nginx", ""));
        unsupported(block_on(fake.upgrade(&upgrade)).err().unwrap().kind());
        unsupported(
            block_on(fake.rollback(&RollbackRequest::new(release_ref(), 1)))
                .unwrap_err()
                .kind(),
        );
        unsupported(
            block_on(fake.uninstall(&UninstallRequest::new(release_ref())))
                .unwrap_err()
                .kind(),
        );
        // Refused mutations are still recorded so a test can assert they were attempted.
        assert_eq!(
            fake.recorded_calls()
                .iter()
                .filter(|c| c.is_mutating())
                .count(),
            4
        );
    }

    #[test]
    fn capabilities_can_change_at_runtime() {
        let fake = FakeHelmPort::native_only();
        fake.script().repos.push_ok(vec![RepoEntry {
            name: "bitnami".into(),
            url: "https://charts.bitnami.com/bitnami".into(),
        }]);
        assert!(block_on(fake.repos()).is_err());
        fake.set_capabilities(HelmCapabilities::full(HelmVersion::new(3, 15, 0)));
        assert_eq!(block_on(fake.repos()).unwrap().len(), 1);
    }

    #[test]
    fn nothing_available_refuses_reads_too() {
        let fake = FakeHelmPort::with_capabilities(HelmCapabilities::none());
        let all = NamespaceSelection::default();
        assert_eq!(
            block_on(fake.list_releases(&cluster(), &all))
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported
        );
    }

    #[test]
    fn port_is_object_safe() {
        let boxed: Box<dyn HelmPort> = Box::new(FakeHelmPort::new());
        assert!(boxed.capabilities().can_install);
    }

    #[test]
    fn repo_calls_are_recorded_and_need_the_binary() {
        let fake = FakeHelmPort::new();
        block_on(fake.repo_add(&RepoAddRequest::new("a", "https://x"))).unwrap();
        block_on(fake.repo_remove("a")).unwrap();
        fake.script.repo_update.push_err(OxiError::internal("boom"));
        assert!(block_on(fake.repo_update(&["a".into()])).is_err());
        assert_eq!(fake.recorded_calls().len(), 3);
        let native = FakeHelmPort::native_only();
        let error = block_on(native.repo_add(&RepoAddRequest::new("a", "https://x"))).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
        assert!(!HelmCall::RepoAdd(RepoAddRequest::new("a", "u")).is_mutating());
    }
}
