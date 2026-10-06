//! Data-plane fakes besides resources: [`FakeDiscoveryPort`], [`FakeTableFeedPort`] and
//! [`FakeLogPort`].

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::log::LogLine;
use oxikube_ports::{
    ClockPort, DiscoveryPort, LogOptions, LogPort, LogStream, ServerVersion, Table, TableBatch,
    TableFeed, TableFeedPort, TableOptions,
};
use parking_lot::Mutex;

use super::FakeClockPort;
use crate::script::{CallLog, Script, StreamGauge, Timeline};

// --- DiscoveryPort -----------------------------------------------------------------------

/// Queued responses for each [`FakeDiscoveryPort`] method.
#[derive(Debug, Default)]
pub struct DiscoveryScripts {
    /// `discover`.
    pub discover: Script<Vec<ResourceKind>>,
    /// `resolve`.
    pub resolve: Script<Option<ResourceKind>>,
    /// `server_version`.
    pub server_version: Script<ServerVersion>,
}

/// One call made on a [`FakeDiscoveryPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryCall {
    /// `discover()`.
    Discover,
    /// `resolve(kind)`.
    Resolve(Gvk),
    /// `server_version()`.
    ServerVersion,
}

/// Fake `DiscoveryPort`.
///
/// Fallbacks: `discover` returns the configured kinds ([`with_kinds`](Self::with_kinds)),
/// `resolve` finds a configured kind with the same `Gvk`, and `server_version` returns the
/// configured version (default `v1.33.0`, `linux/amd64`).
#[derive(Debug)]
pub struct FakeDiscoveryPort {
    script: DiscoveryScripts,
    calls: CallLog<DiscoveryCall>,
    kinds: Mutex<Vec<ResourceKind>>,
    version: Mutex<ServerVersion>,
}

fake_plumbing!(FakeDiscoveryPort, DiscoveryScripts, DiscoveryCall);

impl Default for FakeDiscoveryPort {
    fn default() -> Self {
        Self {
            script: DiscoveryScripts::default(),
            calls: CallLog::default(),
            kinds: Mutex::new(Vec::new()),
            version: Mutex::new(ServerVersion {
                major: "1".into(),
                minor: "33".into(),
                git_version: "v1.33.0".into(),
                platform: "linux/amd64".into(),
            }),
        }
    }
}

impl FakeDiscoveryPort {
    /// A fake that serves no kinds and reports `v1.33.0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the kinds `discover` and `resolve` serve.
    #[must_use]
    pub fn with_kinds(self, kinds: impl IntoIterator<Item = ResourceKind>) -> Self {
        *self.kinds.lock() = kinds.into_iter().collect();
        self
    }

    /// Replaces the kinds `discover` and `resolve` serve (a cluster that gains CRDs).
    pub fn set_kinds(&self, kinds: impl IntoIterator<Item = ResourceKind>) {
        *self.kinds.lock() = kinds.into_iter().collect();
    }

    /// Sets the version `server_version` reports.
    #[must_use]
    pub fn with_version(self, version: ServerVersion) -> Self {
        *self.version.lock() = version;
        self
    }
}

#[async_trait]
impl DiscoveryPort for FakeDiscoveryPort {
    async fn discover(&self) -> OxiResult<Vec<ResourceKind>> {
        self.calls.record(DiscoveryCall::Discover);
        self.script
            .discover
            .next_or_else(|| Ok(self.kinds.lock().clone()))
    }

    async fn resolve(&self, kind: &Gvk) -> OxiResult<Option<ResourceKind>> {
        self.calls.record(DiscoveryCall::Resolve(kind.clone()));
        self.script
            .resolve
            .next_or_else(|| Ok(self.kinds.lock().iter().find(|k| &k.gvk == kind).cloned()))
    }

    async fn server_version(&self) -> OxiResult<ServerVersion> {
        self.calls.record(DiscoveryCall::ServerVersion);
        self.script
            .server_version
            .next_or_else(|| Ok(self.version.lock().clone()))
    }
}

// --- TableFeedPort -----------------------------------------------------------------------

/// Queued responses for each [`FakeTableFeedPort`] method.
#[derive(Debug, Default)]
pub struct TableScripts {
    /// `list_table`.
    pub list_table: Script<Table>,
    /// `table_feed`: each entry is the timeline one feed replays.
    pub table_feed: Script<Timeline<TableBatch>>,
}

/// One call made on a [`FakeTableFeedPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableCall {
    /// `list_table`.
    ListTable {
        /// Kind listed.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Options passed.
        options: TableOptions,
    },
    /// `table_feed`.
    TableFeed {
        /// Kind watched.
        kind: Gvk,
        /// Namespace.
        namespace: Option<String>,
        /// Options passed.
        options: TableOptions,
    },
}

/// Fake `TableFeedPort`. Both methods are scripted only (unscripted calls return the
/// `unscripted` error); feeds replay on [`clock`](Self::clock).
#[derive(Debug)]
pub struct FakeTableFeedPort {
    script: TableScripts,
    calls: CallLog<TableCall>,
    clock: Arc<FakeClockPort>,
    feeds: StreamGauge,
}

fake_plumbing!(FakeTableFeedPort, TableScripts, TableCall);

impl Default for FakeTableFeedPort {
    fn default() -> Self {
        Self::with_clock(Arc::new(FakeClockPort::default()))
    }
}

impl FakeTableFeedPort {
    /// A fake with its own [`FakeClockPort`].
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake whose feeds are timed on `clock`.
    pub fn with_clock(clock: Arc<FakeClockPort>) -> Self {
        Self {
            script: TableScripts::default(),
            calls: CallLog::default(),
            clock,
            feeds: StreamGauge::default(),
        }
    }

    /// The clock feeds are timed on.
    pub fn clock(&self) -> &Arc<FakeClockPort> {
        &self.clock
    }

    /// Table feeds handed out by `table_feed` that the caller has not dropped yet.
    pub fn live_feeds(&self) -> usize {
        self.feeds.live()
    }
}

#[async_trait]
impl TableFeedPort for FakeTableFeedPort {
    async fn list_table(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<Table> {
        self.calls.record(TableCall::ListTable {
            kind: kind.clone(),
            namespace: namespace.map(str::to_owned),
            options: options.clone(),
        });
        self.script
            .list_table
            .next_or_unscripted("FakeTableFeedPort", "list_table")
    }

    async fn table_feed(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<TableFeed> {
        self.calls.record(TableCall::TableFeed {
            kind: kind.clone(),
            namespace: namespace.map(str::to_owned),
            options: options.clone(),
        });
        let timeline = self
            .script
            .table_feed
            .next_or_unscripted("FakeTableFeedPort", "table_feed")?;
        let clock: Arc<dyn ClockPort> = self.clock.clone();
        Ok(self.feeds.track(timeline.replay(clock)))
    }
}

// --- LogPort -----------------------------------------------------------------------------

/// Queued responses for [`FakeLogPort`].
#[derive(Debug, Default)]
pub struct LogScripts {
    /// `stream_logs`: each entry is the timeline one log stream replays.
    pub stream_logs: Script<Timeline<LogLine>>,
}

/// One call made on a [`FakeLogPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogCall {
    /// `stream_logs`.
    StreamLogs {
        /// Pod namespace.
        namespace: String,
        /// Pod name.
        pod: String,
        /// Options passed.
        options: LogOptions,
    },
}

/// Fake `LogPort`. Scripted only; streams replay on [`clock`](Self::clock).
#[derive(Debug)]
pub struct FakeLogPort {
    script: LogScripts,
    calls: CallLog<LogCall>,
    clock: Arc<FakeClockPort>,
}

fake_plumbing!(FakeLogPort, LogScripts, LogCall);

impl Default for FakeLogPort {
    fn default() -> Self {
        Self::with_clock(Arc::new(FakeClockPort::default()))
    }
}

impl FakeLogPort {
    /// A fake with its own [`FakeClockPort`].
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake whose streams are timed on `clock`.
    pub fn with_clock(clock: Arc<FakeClockPort>) -> Self {
        Self {
            script: LogScripts::default(),
            calls: CallLog::default(),
            clock,
        }
    }

    /// The clock streams are timed on.
    pub fn clock(&self) -> &Arc<FakeClockPort> {
        &self.clock
    }
}

#[async_trait]
impl LogPort for FakeLogPort {
    async fn stream_logs(
        &self,
        namespace: &str,
        pod: &str,
        options: &LogOptions,
    ) -> OxiResult<LogStream> {
        self.calls.record(LogCall::StreamLogs {
            namespace: namespace.to_owned(),
            pod: pod.to_owned(),
            options: options.clone(),
        });
        let timeline = self
            .script
            .stream_logs
            .next_or_unscripted("FakeLogPort", "stream_logs")?;
        let clock: Arc<dyn ClockPort> = self.clock.clone();
        Ok(timeline.replay(clock))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use futures::executor::block_on;
    use futures::{FutureExt, StreamExt};
    use oxikube_domain::kinds::VerbSet;
    use oxikube_domain::{ErrorKind, OxiError};
    use oxikube_ports::{Delta, DeltaBatch, TableColumn, TableRow};

    fn pod_kind() -> ResourceKind {
        ResourceKind {
            gvk: Gvk::new("", "v1", "Pod"),
            preferred: true,
            plural: "pods".into(),
            singular: "pod".into(),
            short_names: vec!["po".into()],
            categories: vec!["all".into()],
            verbs: VerbSet::from_names(["get", "list", "watch"]),
            namespaced: true,
        }
    }

    #[test]
    fn discovery_scripts_then_falls_back_to_configured_kinds() {
        let fake = FakeDiscoveryPort::new().with_kinds([pod_kind()]);
        fake.script()
            .discover
            .push_ok(Vec::new())
            .push_err(OxiError::network("down"));
        assert!(block_on(fake.discover()).unwrap().is_empty());
        assert_eq!(
            block_on(fake.discover()).unwrap_err().kind(),
            ErrorKind::Network
        );
        assert_eq!(block_on(fake.discover()).unwrap(), vec![pod_kind()]);
        let gvk = Gvk::new("", "v1", "Pod");
        assert_eq!(block_on(fake.resolve(&gvk)).unwrap(), Some(pod_kind()));
        assert_eq!(
            block_on(fake.server_version()).unwrap().numeric(),
            Some((1, 33))
        );
        assert_eq!(
            fake.recorded_calls(),
            vec![
                DiscoveryCall::Discover,
                DiscoveryCall::Discover,
                DiscoveryCall::Discover,
                DiscoveryCall::Resolve(gvk),
                DiscoveryCall::ServerVersion,
            ]
        );
    }

    #[test]
    fn table_feed_scripts_ok_and_err_and_replays_on_the_clock() {
        let fake = FakeTableFeedPort::new();
        let kind = Gvk::new("", "v1", "Pod");
        let table = Table {
            columns: Arc::from(vec![TableColumn {
                name: "Name".into(),
                ..TableColumn::default()
            }]),
            ..Table::default()
        };
        fake.script()
            .list_table
            .push_ok(table.clone())
            .push_err(OxiError::forbidden("no"));
        assert_eq!(
            block_on(fake.list_table(&kind, None, &TableOptions::default())).unwrap(),
            table
        );
        assert_eq!(
            block_on(fake.list_table(&kind, None, &TableOptions::default()))
                .unwrap_err()
                .kind(),
            ErrorKind::Forbidden
        );

        let row = TableRow::default();
        fake.script().table_feed.push_ok(Timeline::new().ok_at(
            Duration::from_secs(2),
            TableBatch {
                columns: None,
                rows: DeltaBatch::from_deltas(vec![Delta::Applied(row)]),
                source: Default::default(),
            },
        ));
        let mut feed =
            block_on(fake.table_feed(&kind, Some("demo"), &TableOptions::default())).unwrap();
        assert!(feed.next().now_or_never().is_none());
        fake.clock().advance(Duration::from_secs(2));
        assert_eq!(block_on(feed.next()).unwrap().unwrap().rows.len(), 1);
        assert_eq!(fake.recorded_calls().len(), 3);
        assert!(matches!(
            &fake.recorded_calls()[2],
            TableCall::TableFeed { namespace: Some(ns), .. } if ns == "demo"
        ));
        // Nothing scripted any more: the call itself fails.
        assert!(block_on(fake.table_feed(&kind, None, &TableOptions::default())).is_err());
    }

    #[test]
    fn log_stream_replays_lines_and_records_options() {
        let fake = FakeLogPort::new();
        let t0 = fake.clock().now();
        let line = |text: &str| LogLine::new(t0, "web", "app", text);
        fake.script()
            .stream_logs
            .push_ok(
                Timeline::immediate([line("one")])
                    .err_at(Duration::from_secs(1), OxiError::network("reset")),
            )
            .push_err(OxiError::not_found("pod gone"));
        let options = LogOptions::follow().container("app").tail_lines(10);
        let mut stream = block_on(fake.stream_logs("demo", "web", &options)).unwrap();
        assert_eq!(block_on(stream.next()).unwrap().unwrap().text, "one");
        assert!(stream.next().now_or_never().is_none());
        fake.clock().advance(Duration::from_secs(1));
        assert!(block_on(stream.next()).unwrap().unwrap_err().is_retryable());
        assert_eq!(
            block_on(fake.stream_logs("demo", "web", &options))
                .err()
                .map(|e| e.kind()),
            Some(ErrorKind::NotFound)
        );
        assert_eq!(
            fake.recorded_calls()[0],
            LogCall::StreamLogs {
                namespace: "demo".into(),
                pod: "web".into(),
                options,
            }
        );
    }
}
