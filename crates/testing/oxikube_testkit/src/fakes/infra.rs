//! Infrastructure fakes for scripted, mostly read-only ports: [`FakeMetricsPort`],
//! [`FakePromqlPort`], [`FakeDescribePort`], [`FakeHelmPort`], [`FakeNotifierPort`],
//! [`FakeUpdaterPort`] and [`FakeCrashReporterPort`].

use async_trait::async_trait;
use jiff::Timestamp;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{
    CrashId, CrashReport, CrashReporterPort, DescribeOutput, DescribePort, DownloadedUpdate,
    HelmPort, HelmRelease, HelmReleaseRef, MetricsOutcome, MetricsPort, Notification, NotifierPort,
    PromqlPort, PromqlSeries, TimeRange, UpdateChannel, UpdateInfo, UpdaterPort,
};
use parking_lot::Mutex;

use crate::script::{CallLog, Script};

// --- MetricsPort -------------------------------------------------------------------------

/// Queued responses for each [`FakeMetricsPort`] method.
#[derive(Debug, Default)]
pub struct MetricsScripts {
    /// `node_metrics`.
    pub node_metrics: Script<MetricsOutcome>,
    /// `pod_metrics`.
    pub pod_metrics: Script<MetricsOutcome>,
}

/// One call made on a [`FakeMetricsPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricsCall {
    /// `node_metrics(cluster)`.
    NodeMetrics(ClusterId),
    /// `pod_metrics(cluster, namespaces)`.
    PodMetrics(ClusterId, NamespaceSelection),
}

/// Fake `MetricsPort`. Scripted only.
#[derive(Debug, Default)]
pub struct FakeMetricsPort {
    script: MetricsScripts,
    calls: CallLog<MetricsCall>,
}

fake_plumbing!(FakeMetricsPort, MetricsScripts, MetricsCall);

impl FakeMetricsPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MetricsPort for FakeMetricsPort {
    async fn node_metrics(&self, cluster: &ClusterId) -> OxiResult<MetricsOutcome> {
        self.calls.record(MetricsCall::NodeMetrics(cluster.clone()));
        self.script
            .node_metrics
            .next_or_unscripted("FakeMetricsPort", "node_metrics")
    }

    async fn pod_metrics(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<MetricsOutcome> {
        self.calls
            .record(MetricsCall::PodMetrics(cluster.clone(), namespaces.clone()));
        self.script
            .pod_metrics
            .next_or_unscripted("FakeMetricsPort", "pod_metrics")
    }
}

// --- PromqlPort --------------------------------------------------------------------------

/// Queued responses for each [`FakePromqlPort`] method.
#[derive(Debug, Default)]
pub struct PromqlScripts {
    /// `is_available`.
    pub is_available: Script<bool>,
    /// `query_instant`.
    pub query_instant: Script<Vec<PromqlSeries>>,
    /// `query_range`.
    pub query_range: Script<Vec<PromqlSeries>>,
}

/// One call made on a [`FakePromqlPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromqlCall {
    /// `is_available(cluster)`.
    IsAvailable(ClusterId),
    /// `query_instant`.
    QueryInstant {
        /// Cluster queried.
        cluster: ClusterId,
        /// PromQL text.
        query: String,
        /// Evaluation time.
        at: Option<Timestamp>,
    },
    /// `query_range`.
    QueryRange {
        /// Cluster queried.
        cluster: ClusterId,
        /// PromQL text.
        query: String,
        /// Range and step.
        range: TimeRange,
    },
}

/// Fake `PromqlPort`. Fallback: `is_available` is `false` (no Prometheus); queries are
/// scripted only.
#[derive(Debug, Default)]
pub struct FakePromqlPort {
    script: PromqlScripts,
    calls: CallLog<PromqlCall>,
}

fake_plumbing!(FakePromqlPort, PromqlScripts, PromqlCall);

impl FakePromqlPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl PromqlPort for FakePromqlPort {
    async fn is_available(&self, cluster: &ClusterId) -> OxiResult<bool> {
        self.calls.record(PromqlCall::IsAvailable(cluster.clone()));
        self.script.is_available.next_or_else(|| Ok(false))
    }

    async fn query_instant(
        &self,
        cluster: &ClusterId,
        query: &str,
        at: Option<Timestamp>,
    ) -> OxiResult<Vec<PromqlSeries>> {
        self.calls.record(PromqlCall::QueryInstant {
            cluster: cluster.clone(),
            query: query.to_owned(),
            at,
        });
        self.script
            .query_instant
            .next_or_unscripted("FakePromqlPort", "query_instant")
    }

    async fn query_range(
        &self,
        cluster: &ClusterId,
        query: &str,
        range: TimeRange,
    ) -> OxiResult<Vec<PromqlSeries>> {
        self.calls.record(PromqlCall::QueryRange {
            cluster: cluster.clone(),
            query: query.to_owned(),
            range,
        });
        self.script
            .query_range
            .next_or_unscripted("FakePromqlPort", "query_range")
    }
}

// --- DescribePort ------------------------------------------------------------------------

/// Queued responses for [`FakeDescribePort`].
#[derive(Debug, Default)]
pub struct DescribeScripts {
    /// `describe`.
    pub describe: Script<DescribeOutput>,
}

/// One call made on a [`FakeDescribePort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescribeCall {
    /// `describe(target)`.
    Describe(ResourceRef),
}

/// Fake `DescribePort`. Scripted only.
#[derive(Debug, Default)]
pub struct FakeDescribePort {
    script: DescribeScripts,
    calls: CallLog<DescribeCall>,
}

fake_plumbing!(FakeDescribePort, DescribeScripts, DescribeCall);

impl FakeDescribePort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl DescribePort for FakeDescribePort {
    async fn describe(&self, target: &ResourceRef) -> OxiResult<DescribeOutput> {
        self.calls.record(DescribeCall::Describe(target.clone()));
        self.script
            .describe
            .next_or_unscripted("FakeDescribePort", "describe")
    }
}

// --- HelmPort ----------------------------------------------------------------------------

/// Queued responses for each [`FakeHelmPort`] method.
#[derive(Debug, Default)]
pub struct HelmScripts {
    /// `list_releases`.
    pub list_releases: Script<Vec<HelmRelease>>,
    /// `history`.
    pub history: Script<Vec<HelmRelease>>,
    /// `values`.
    pub values: Script<String>,
    /// `manifest`.
    pub manifest: Script<String>,
    /// `rollback` (mutating).
    pub rollback: Script<()>,
    /// `uninstall` (mutating).
    pub uninstall: Script<()>,
}

/// One call made on a [`FakeHelmPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelmCall {
    /// `list_releases`.
    ListReleases(ClusterId, NamespaceSelection),
    /// `history`.
    History(HelmReleaseRef),
    /// `values`.
    Values {
        /// Release.
        release: HelmReleaseRef,
        /// Revision, `None` for the latest.
        revision: Option<u32>,
        /// Computed values instead of user-supplied ones.
        all: bool,
    },
    /// `manifest`.
    Manifest {
        /// Release.
        release: HelmReleaseRef,
        /// Revision, `None` for the latest.
        revision: Option<u32>,
    },
    /// `rollback` (mutating).
    Rollback {
        /// Release.
        release: HelmReleaseRef,
        /// Target revision.
        revision: u32,
    },
    /// `uninstall` (mutating).
    Uninstall {
        /// Release.
        release: HelmReleaseRef,
        /// `--keep-history`.
        keep_history: bool,
    },
}

impl HelmCall {
    /// `true` for `rollback` and `uninstall`, the calls `MutationGuard` must gate.
    pub fn is_mutating(&self) -> bool {
        matches!(self, Self::Rollback { .. } | Self::Uninstall { .. })
    }
}

/// Fake `HelmPort`. Fallback: `list_releases` is empty; everything else is scripted
/// only.
#[derive(Debug, Default)]
pub struct FakeHelmPort {
    script: HelmScripts,
    calls: CallLog<HelmCall>,
}

fake_plumbing!(FakeHelmPort, HelmScripts, HelmCall);

impl FakeHelmPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl HelmPort for FakeHelmPort {
    async fn list_releases(
        &self,
        cluster: &ClusterId,
        namespaces: &NamespaceSelection,
    ) -> OxiResult<Vec<HelmRelease>> {
        self.calls
            .record(HelmCall::ListReleases(cluster.clone(), namespaces.clone()));
        self.script.list_releases.next_or_else(|| Ok(Vec::new()))
    }

    async fn history(&self, release: &HelmReleaseRef) -> OxiResult<Vec<HelmRelease>> {
        self.calls.record(HelmCall::History(release.clone()));
        self.script
            .history
            .next_or_unscripted("FakeHelmPort", "history")
    }

    async fn values(
        &self,
        release: &HelmReleaseRef,
        revision: Option<u32>,
        all: bool,
    ) -> OxiResult<String> {
        self.calls.record(HelmCall::Values {
            release: release.clone(),
            revision,
            all,
        });
        self.script
            .values
            .next_or_unscripted("FakeHelmPort", "values")
    }

    async fn manifest(&self, release: &HelmReleaseRef, revision: Option<u32>) -> OxiResult<String> {
        self.calls.record(HelmCall::Manifest {
            release: release.clone(),
            revision,
        });
        self.script
            .manifest
            .next_or_unscripted("FakeHelmPort", "manifest")
    }

    async fn rollback(&self, release: &HelmReleaseRef, revision: u32) -> OxiResult<()> {
        self.calls.record(HelmCall::Rollback {
            release: release.clone(),
            revision,
        });
        self.script
            .rollback
            .next_or_unscripted("FakeHelmPort", "rollback")
    }

    async fn uninstall(&self, release: &HelmReleaseRef, keep_history: bool) -> OxiResult<()> {
        self.calls.record(HelmCall::Uninstall {
            release: release.clone(),
            keep_history,
        });
        self.script
            .uninstall
            .next_or_unscripted("FakeHelmPort", "uninstall")
    }
}

// --- NotifierPort ------------------------------------------------------------------------

/// Queued responses for each [`FakeNotifierPort`] method.
#[derive(Debug, Default)]
pub struct NotifierScripts {
    /// `notify`.
    pub notify: Script<()>,
    /// `clear`.
    pub clear: Script<()>,
}

/// One call made on a [`FakeNotifierPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotifierCall {
    /// `notify(notification)`.
    Notify(Notification),
    /// `clear(tag)`.
    Clear(String),
}

/// Fake `NotifierPort`. Fallback: both methods succeed; the notifications are in
/// [`recorded_calls`](Self::recorded_calls) and [`notifications`](Self::notifications).
#[derive(Debug, Default)]
pub struct FakeNotifierPort {
    script: NotifierScripts,
    calls: CallLog<NotifierCall>,
}

fake_plumbing!(FakeNotifierPort, NotifierScripts, NotifierCall);

impl FakeNotifierPort {
    /// A fake that accepts every notification.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every notification passed to `notify`, in order (including ones whose scripted
    /// response was an error).
    pub fn notifications(&self) -> Vec<Notification> {
        self.calls
            .calls()
            .into_iter()
            .filter_map(|c| match c {
                NotifierCall::Notify(n) => Some(n),
                NotifierCall::Clear(_) => None,
            })
            .collect()
    }
}

#[async_trait]
impl NotifierPort for FakeNotifierPort {
    async fn notify(&self, notification: &Notification) -> OxiResult<()> {
        self.calls
            .record(NotifierCall::Notify(notification.clone()));
        self.script.notify.next_or_else(|| Ok(()))
    }

    async fn clear(&self, tag: &str) -> OxiResult<()> {
        self.calls.record(NotifierCall::Clear(tag.to_owned()));
        self.script.clear.next_or_else(|| Ok(()))
    }
}

// --- UpdaterPort -------------------------------------------------------------------------

/// Queued responses for each [`FakeUpdaterPort`] method.
#[derive(Debug, Default)]
pub struct UpdaterScripts {
    /// `check`.
    pub check: Script<Option<UpdateInfo>>,
    /// `download`.
    pub download: Script<DownloadedUpdate>,
    /// `install`.
    pub install: Script<()>,
}

/// One call made on a [`FakeUpdaterPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdaterCall {
    /// `check(current_version, channel)`.
    Check(String, UpdateChannel),
    /// `download(update)`.
    Download(UpdateInfo),
    /// `install(update)`.
    Install(DownloadedUpdate),
}

/// Fake `UpdaterPort`. Fallback: `check` finds no update; `download` and `install` are
/// scripted only.
#[derive(Debug, Default)]
pub struct FakeUpdaterPort {
    script: UpdaterScripts,
    calls: CallLog<UpdaterCall>,
}

fake_plumbing!(FakeUpdaterPort, UpdaterScripts, UpdaterCall);

impl FakeUpdaterPort {
    /// A fake that never finds an update.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl UpdaterPort for FakeUpdaterPort {
    async fn check(
        &self,
        current_version: &str,
        channel: UpdateChannel,
    ) -> OxiResult<Option<UpdateInfo>> {
        self.calls
            .record(UpdaterCall::Check(current_version.to_owned(), channel));
        self.script.check.next_or_else(|| Ok(None))
    }

    async fn download(&self, update: &UpdateInfo) -> OxiResult<DownloadedUpdate> {
        self.calls.record(UpdaterCall::Download(update.clone()));
        self.script
            .download
            .next_or_unscripted("FakeUpdaterPort", "download")
    }

    async fn install(&self, update: &DownloadedUpdate) -> OxiResult<()> {
        self.calls.record(UpdaterCall::Install(update.clone()));
        self.script
            .install
            .next_or_unscripted("FakeUpdaterPort", "install")
    }
}

// --- CrashReporterPort -------------------------------------------------------------------

/// Queued responses for each [`FakeCrashReporterPort`] method.
#[derive(Debug, Default)]
pub struct CrashScripts {
    /// `record`.
    pub record: Script<()>,
    /// `pending`.
    pub pending: Script<Vec<CrashReport>>,
    /// `submit`.
    pub submit: Script<()>,
    /// `discard`.
    pub discard: Script<bool>,
}

/// One call made on a [`FakeCrashReporterPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CrashCall {
    /// `record(report)`.
    Record(CrashId),
    /// `pending()`.
    Pending,
    /// `submit(id)`.
    Submit(CrashId),
    /// `discard(id)`.
    Discard(CrashId),
}

/// Fake `CrashReporterPort` backed by an in-memory list of reports.
///
/// Fallbacks: `record` stores the report, `pending` lists the stored ones, `submit`
/// removes it (adding it to [`submitted`](Self::submitted); `NotFound` if unknown) and
/// `discard` removes it, returning whether it existed.
#[derive(Debug, Default)]
pub struct FakeCrashReporterPort {
    script: CrashScripts,
    calls: CallLog<CrashCall>,
    reports: Mutex<Vec<CrashReport>>,
    submitted: Mutex<Vec<CrashReport>>,
}

fake_plumbing!(FakeCrashReporterPort, CrashScripts, CrashCall);

impl FakeCrashReporterPort {
    /// A fake with no stored reports.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reports submitted so far, in order.
    pub fn submitted(&self) -> Vec<CrashReport> {
        self.submitted.lock().clone()
    }

    fn take(&self, id: &CrashId) -> Option<CrashReport> {
        let mut reports = self.reports.lock();
        let at = reports.iter().position(|r| &r.id == id)?;
        Some(reports.remove(at))
    }
}

#[async_trait]
impl CrashReporterPort for FakeCrashReporterPort {
    async fn record(&self, report: &CrashReport) -> OxiResult<()> {
        self.calls.record(CrashCall::Record(report.id.clone()));
        self.script.record.next_or_else(|| {
            self.reports.lock().push(report.clone());
            Ok(())
        })
    }

    async fn pending(&self) -> OxiResult<Vec<CrashReport>> {
        self.calls.record(CrashCall::Pending);
        self.script
            .pending
            .next_or_else(|| Ok(self.reports.lock().clone()))
    }

    async fn submit(&self, id: &CrashId) -> OxiResult<()> {
        self.calls.record(CrashCall::Submit(id.clone()));
        self.script.submit.next_or_else(|| {
            let report = self
                .take(id)
                .ok_or_else(|| OxiError::not_found(format!("crash report {}", id.0)))?;
            self.submitted.lock().push(report);
            Ok(())
        })
    }

    async fn discard(&self, id: &CrashId) -> OxiResult<bool> {
        self.calls.record(CrashCall::Discard(id.clone()));
        self.script
            .discard
            .next_or_else(|| Ok(self.take(id).is_some()))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use futures::executor::block_on;
    use oxikube_domain::ErrorKind;
    use oxikube_domain::ids::{ContextName, Gvk};
    use oxikube_domain::metrics::MissingReason;
    use oxikube_ports::{DescribeSource, HelmReleaseStatus, NotificationLevel};

    fn cluster() -> ClusterId {
        ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
    }

    fn t0() -> Timestamp {
        "2026-01-01T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn metrics_scripted_ok_err_and_unscripted() {
        let fake = FakeMetricsPort::new();
        fake.script()
            .node_metrics
            .push_ok(MetricsOutcome::Unavailable(MissingReason::NotInstalled))
            .push_err(OxiError::forbidden("no metrics"));
        let first = block_on(fake.node_metrics(&cluster())).unwrap();
        assert_eq!(
            first.unavailable_reason(),
            Some(MissingReason::NotInstalled)
        );
        assert_eq!(
            block_on(fake.node_metrics(&cluster())).unwrap_err().kind(),
            ErrorKind::Forbidden
        );
        let all = NamespaceSelection::default();
        assert_eq!(
            block_on(fake.pod_metrics(&cluster(), &all))
                .unwrap_err()
                .kind(),
            ErrorKind::Internal
        );
        assert_eq!(
            fake.recorded_calls(),
            vec![
                MetricsCall::NodeMetrics(cluster()),
                MetricsCall::NodeMetrics(cluster()),
                MetricsCall::PodMetrics(cluster(), all),
            ]
        );
    }

    #[test]
    fn promql_scripted_ok_err_and_fallback() {
        let fake = FakePromqlPort::new();
        assert!(!block_on(fake.is_available(&cluster())).unwrap());
        let series = PromqlSeries {
            labels: [("pod".to_owned(), "web".to_owned())].into(),
            points: Vec::new(),
        };
        fake.script()
            .query_range
            .push_ok(vec![series.clone()])
            .push_err(OxiError::timeout("slow"));
        let range = TimeRange {
            start: t0(),
            end: t0(),
            step: Duration::from_secs(15),
        };
        assert_eq!(
            block_on(fake.query_range(&cluster(), "up", range)).unwrap(),
            vec![series]
        );
        assert_eq!(
            block_on(fake.query_range(&cluster(), "up", range))
                .unwrap_err()
                .kind(),
            ErrorKind::Timeout
        );
        assert!(block_on(fake.query_instant(&cluster(), "up", None)).is_err());
        assert_eq!(fake.recorded_calls().len(), 4);
        assert_eq!(
            fake.recorded_calls()[1],
            PromqlCall::QueryRange {
                cluster: cluster(),
                query: "up".into(),
                range
            }
        );
    }

    #[test]
    fn describe_scripted_ok_and_err() {
        let fake = FakeDescribePort::new();
        let target = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "demo", "web");
        fake.script()
            .describe
            .push_ok(DescribeOutput {
                text: "Name: web".into(),
                source: DescribeSource::Native,
            })
            .push_err(OxiError::not_found("gone"));
        assert_eq!(block_on(fake.describe(&target)).unwrap().text, "Name: web");
        assert_eq!(
            block_on(fake.describe(&target)).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            fake.recorded_calls(),
            vec![
                DescribeCall::Describe(target.clone()),
                DescribeCall::Describe(target)
            ]
        );
    }

    #[test]
    fn helm_scripted_ok_err_and_mutations_recorded() {
        let fake = FakeHelmPort::new();
        let release = HelmReleaseRef {
            cluster: cluster(),
            namespace: "demo".into(),
            name: "web".into(),
        };
        let rel = HelmRelease {
            release: release.clone(),
            revision: 2,
            status: HelmReleaseStatus::Deployed,
            chart: "web".into(),
            chart_version: "1.2.3".into(),
            app_version: Some("1.0".into()),
            updated: t0(),
        };
        let all = NamespaceSelection::default();
        assert!(
            block_on(fake.list_releases(&cluster(), &all))
                .unwrap()
                .is_empty()
        );
        fake.script().history.push_ok(vec![rel.clone()]);
        assert_eq!(block_on(fake.history(&release)).unwrap(), vec![rel]);
        fake.script()
            .rollback
            .push_ok(())
            .push_err(OxiError::conflict("in progress"));
        block_on(fake.rollback(&release, 1)).unwrap();
        assert_eq!(
            block_on(fake.rollback(&release, 1)).unwrap_err().kind(),
            ErrorKind::Conflict
        );
        assert!(block_on(fake.values(&release, None, true)).is_err());
        assert!(block_on(fake.manifest(&release, Some(1))).is_err());
        assert!(block_on(fake.uninstall(&release, false)).is_err());
        let calls = fake.recorded_calls();
        assert_eq!(calls.len(), 7);
        assert_eq!(calls.iter().filter(|c| c.is_mutating()).count(), 3);
    }

    #[test]
    fn notifier_succeeds_by_default_and_scripts_errors() {
        let fake = FakeNotifierPort::new();
        let note = Notification {
            title: "Pod crashed".into(),
            body: "web restarted".into(),
            level: NotificationLevel::Warning,
            cluster: None,
            tag: Some("crash/web".into()),
        };
        block_on(fake.notify(&note)).unwrap();
        fake.script()
            .notify
            .push_err(OxiError::unsupported("no daemon"));
        assert!(block_on(fake.notify(&note)).is_err());
        block_on(fake.clear("crash/web")).unwrap();
        assert_eq!(fake.notifications(), vec![note.clone(), note]);
        assert_eq!(
            fake.recorded_calls()[2],
            NotifierCall::Clear("crash/web".into())
        );
    }

    #[test]
    fn updater_scripted_ok_err_and_fallback() {
        let fake = FakeUpdaterPort::new();
        assert_eq!(
            block_on(fake.check("0.1.0", UpdateChannel::Stable)).unwrap(),
            None
        );
        let info = UpdateInfo {
            version: "0.2.0".into(),
            channel: UpdateChannel::Beta,
            release_notes: None,
            published: None,
            size_bytes: Some(1),
        };
        fake.script().check.push_ok(Some(info.clone()));
        fake.script()
            .download
            .push_err(OxiError::network("offline"));
        assert_eq!(
            block_on(fake.check("0.1.0", UpdateChannel::Beta)).unwrap(),
            Some(info.clone())
        );
        assert_eq!(
            block_on(fake.download(&info)).unwrap_err().kind(),
            ErrorKind::Network
        );
        assert_eq!(
            fake.recorded_calls()[1],
            UpdaterCall::Check("0.1.0".into(), UpdateChannel::Beta)
        );
        assert_eq!(fake.recorded_calls()[2], UpdaterCall::Download(info));
    }

    #[test]
    fn crash_reporter_stores_submits_and_scripts_errors() {
        let fake = FakeCrashReporterPort::new();
        let report = |id: &str| CrashReport {
            id: CrashId(id.into()),
            ts: t0(),
            app_version: "0.1.0".into(),
            summary: "panic".into(),
            backtrace: String::new(),
        };
        block_on(fake.record(&report("a"))).unwrap();
        block_on(fake.record(&report("b"))).unwrap();
        fake.script()
            .record
            .push_err(OxiError::internal("disk full"));
        assert!(block_on(fake.record(&report("c"))).is_err());
        assert_eq!(block_on(fake.pending()).unwrap().len(), 2);
        block_on(fake.submit(&CrashId("a".into()))).unwrap();
        assert_eq!(fake.submitted(), vec![report("a")]);
        assert_eq!(
            block_on(fake.submit(&CrashId("zzz".into())))
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert!(block_on(fake.discard(&CrashId("b".into()))).unwrap());
        assert!(!block_on(fake.discard(&CrashId("b".into()))).unwrap());
        assert_eq!(
            fake.recorded_calls()[0],
            CrashCall::Record(CrashId("a".into()))
        );
        assert_eq!(fake.recorded_calls().len(), 8);
    }
}
