//! In-memory storage fakes: [`FakeStatePort`], [`FakeSecretStorePort`] and
//! [`FakeFsPort`]. Nothing touches the disk or the OS keychain.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use oxikube_domain::audit::AuditRecord;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::secrets::SecretString;
use oxikube_ports::{
    AuditQuery, DirEntry, EntryKind, FsEvent, FsEventKind, FsPort, SecretKey, SecretStorePort,
    StateKey, StatePort, StateTable,
};
use parking_lot::Mutex;
use serde_json::Value;

use crate::script::{CallLog, Script};

// --- StatePort ---------------------------------------------------------------------------

/// Queued responses for each [`FakeStatePort`] method.
#[derive(Debug, Default)]
pub struct StateScripts {
    /// `kv_get`.
    pub kv_get: Script<Option<Value>>,
    /// `kv_set`.
    pub kv_set: Script<()>,
    /// `kv_delete`.
    pub kv_delete: Script<bool>,
    /// `kv_list`.
    pub kv_list: Script<Vec<(StateKey, Value)>>,
    /// `table_get`.
    pub table_get: Script<Option<Value>>,
    /// `table_put`.
    pub table_put: Script<()>,
    /// `table_delete`.
    pub table_delete: Script<bool>,
    /// `table_list`.
    pub table_list: Script<Vec<(StateKey, Value)>>,
    /// `append_audit`.
    pub append_audit: Script<()>,
    /// `query_audit`.
    pub query_audit: Script<Vec<AuditRecord>>,
}

/// One call made on a [`FakeStatePort`].
#[derive(Debug, Clone, PartialEq)]
pub enum StateCall {
    /// `kv_get(key)`.
    KvGet(StateKey),
    /// `kv_set(key, value)`.
    KvSet(StateKey, Value),
    /// `kv_delete(key)`.
    KvDelete(StateKey),
    /// `kv_list(prefix)`.
    KvList(String),
    /// `table_get(table, key)`.
    TableGet(StateTable, StateKey),
    /// `table_put(table, key, row)`.
    TablePut(StateTable, StateKey, Value),
    /// `table_delete(table, key)`.
    TableDelete(StateTable, StateKey),
    /// `table_list(table, limit)`.
    TableList(StateTable, Option<usize>),
    /// `append_audit(records)`.
    AppendAudit(Vec<AuditRecord>),
    /// `query_audit(query)`.
    QueryAudit(AuditQuery),
}

#[derive(Default)]
struct StateData {
    kv: BTreeMap<StateKey, Value>,
    tables: BTreeMap<StateTable, BTreeMap<StateKey, Value>>,
    audit: Vec<AuditRecord>,
}

/// Fake `StatePort` over in-memory maps, with the same semantics as the SQLite adapter:
/// lists ordered by key, `kv_list` by prefix, audit append-only and queried newest first
/// (`since` inclusive, `until` exclusive, then `limit`). A scripted response replaces the
/// in-memory behaviour for that call (the store is left unchanged).
#[derive(Default)]
pub struct FakeStatePort {
    script: StateScripts,
    calls: CallLog<StateCall>,
    data: Mutex<StateData>,
}

fake_plumbing!(FakeStatePort, StateScripts, StateCall);

impl std::fmt::Debug for FakeStatePort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = self.data.lock();
        f.debug_struct("FakeStatePort")
            .field("kv", &data.kv.len())
            .field("tables", &data.tables.len())
            .field("audit", &data.audit.len())
            .finish_non_exhaustive()
    }
}

impl FakeStatePort {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every audit record appended so far, in append order.
    pub fn audit_log(&self) -> Vec<AuditRecord> {
        self.data.lock().audit.clone()
    }
}

#[async_trait]
impl StatePort for FakeStatePort {
    async fn kv_get(&self, key: &StateKey) -> OxiResult<Option<Value>> {
        self.calls.record(StateCall::KvGet(key.clone()));
        self.script
            .kv_get
            .next_or_else(|| Ok(self.data.lock().kv.get(key).cloned()))
    }

    async fn kv_set(&self, key: &StateKey, value: Value) -> OxiResult<()> {
        self.calls
            .record(StateCall::KvSet(key.clone(), value.clone()));
        self.script.kv_set.next_or_else(|| {
            self.data.lock().kv.insert(key.clone(), value);
            Ok(())
        })
    }

    async fn kv_delete(&self, key: &StateKey) -> OxiResult<bool> {
        self.calls.record(StateCall::KvDelete(key.clone()));
        self.script
            .kv_delete
            .next_or_else(|| Ok(self.data.lock().kv.remove(key).is_some()))
    }

    async fn kv_list(&self, prefix: &str) -> OxiResult<Vec<(StateKey, Value)>> {
        self.calls.record(StateCall::KvList(prefix.to_owned()));
        self.script.kv_list.next_or_else(|| {
            Ok(self
                .data
                .lock()
                .kv
                .iter()
                .filter(|(k, _)| k.as_str().starts_with(prefix))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect())
        })
    }

    async fn table_get(&self, table: &StateTable, key: &StateKey) -> OxiResult<Option<Value>> {
        self.calls
            .record(StateCall::TableGet(table.clone(), key.clone()));
        self.script.table_get.next_or_else(|| {
            Ok(self
                .data
                .lock()
                .tables
                .get(table)
                .and_then(|t| t.get(key))
                .cloned())
        })
    }

    async fn table_put(&self, table: &StateTable, key: &StateKey, row: Value) -> OxiResult<()> {
        self.calls
            .record(StateCall::TablePut(table.clone(), key.clone(), row.clone()));
        self.script.table_put.next_or_else(|| {
            self.data
                .lock()
                .tables
                .entry(table.clone())
                .or_default()
                .insert(key.clone(), row);
            Ok(())
        })
    }

    async fn table_delete(&self, table: &StateTable, key: &StateKey) -> OxiResult<bool> {
        self.calls
            .record(StateCall::TableDelete(table.clone(), key.clone()));
        self.script.table_delete.next_or_else(|| {
            Ok(self
                .data
                .lock()
                .tables
                .get_mut(table)
                .is_some_and(|t| t.remove(key).is_some()))
        })
    }

    async fn table_list(
        &self,
        table: &StateTable,
        limit: Option<usize>,
    ) -> OxiResult<Vec<(StateKey, Value)>> {
        self.calls
            .record(StateCall::TableList(table.clone(), limit));
        self.script.table_list.next_or_else(|| {
            let data = self.data.lock();
            let rows = data.tables.get(table).into_iter().flatten();
            Ok(rows
                .take(limit.unwrap_or(usize::MAX))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect())
        })
    }

    async fn append_audit(&self, records: &[AuditRecord]) -> OxiResult<()> {
        self.calls.record(StateCall::AppendAudit(records.to_vec()));
        self.script.append_audit.next_or_else(|| {
            self.data.lock().audit.extend_from_slice(records);
            Ok(())
        })
    }

    async fn query_audit(&self, query: &AuditQuery) -> OxiResult<Vec<AuditRecord>> {
        self.calls.record(StateCall::QueryAudit(query.clone()));
        self.script.query_audit.next_or_else(|| {
            let data = self.data.lock();
            let mut hits: Vec<AuditRecord> = data
                .audit
                .iter()
                .filter(|r| query.cluster.as_ref().is_none_or(|c| &r.cluster == c))
                .filter(|r| query.since.is_none_or(|s| r.ts >= s))
                .filter(|r| query.until.is_none_or(|u| r.ts < u))
                .filter(|r| query.cmd.as_deref().is_none_or(|c| &*r.cmd == c))
                .cloned()
                .collect();
            // Newest first; equal timestamps keep "last appended first".
            hits.reverse();
            hits.sort_by_key(|r| std::cmp::Reverse(r.ts));
            hits.truncate(query.limit);
            Ok(hits)
        })
    }
}

// --- SecretStorePort ---------------------------------------------------------------------

/// Queued responses for each [`FakeSecretStorePort`] method.
#[derive(Debug, Default)]
pub struct SecretScripts {
    /// `get`.
    pub get: Script<Option<SecretString>>,
    /// `set`.
    pub set: Script<()>,
    /// `delete`.
    pub delete: Script<bool>,
}

/// One call made on a [`FakeSecretStorePort`]. Secret values are never recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretCall {
    /// `get(key)`.
    Get(SecretKey),
    /// `set(key, _)`.
    Set(SecretKey),
    /// `delete(key)`.
    Delete(SecretKey),
}

/// Fake `SecretStorePort`: secrets live only in memory, as `SecretString`s (never on
/// disk, never in `Debug` output or recorded calls). A scripted response replaces the
/// in-memory behaviour for that call.
#[derive(Default)]
pub struct FakeSecretStorePort {
    script: SecretScripts,
    calls: CallLog<SecretCall>,
    secrets: Mutex<BTreeMap<SecretKey, SecretString>>,
}

fake_plumbing!(FakeSecretStorePort, SecretScripts, SecretCall);

impl std::fmt::Debug for FakeSecretStorePort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeSecretStorePort")
            .field("keys", &self.keys())
            .finish_non_exhaustive()
    }
}

impl FakeSecretStorePort {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// The keys currently stored, ordered.
    pub fn keys(&self) -> Vec<SecretKey> {
        self.secrets.lock().keys().cloned().collect()
    }

    /// The secret stored under `key`, without recording a call.
    pub fn peek(&self, key: &SecretKey) -> Option<SecretString> {
        self.secrets.lock().get(key).cloned()
    }
}

#[async_trait]
impl SecretStorePort for FakeSecretStorePort {
    async fn get(&self, key: &SecretKey) -> OxiResult<Option<SecretString>> {
        self.calls.record(SecretCall::Get(key.clone()));
        self.script
            .get
            .next_or_else(|| Ok(self.secrets.lock().get(key).cloned()))
    }

    async fn set(&self, key: &SecretKey, value: SecretString) -> OxiResult<()> {
        self.calls.record(SecretCall::Set(key.clone()));
        self.script.set.next_or_else(|| {
            self.secrets.lock().insert(key.clone(), value);
            Ok(())
        })
    }

    async fn delete(&self, key: &SecretKey) -> OxiResult<bool> {
        self.calls.record(SecretCall::Delete(key.clone()));
        self.script
            .delete
            .next_or_else(|| Ok(self.secrets.lock().remove(key).is_some()))
    }
}

// --- FsPort ------------------------------------------------------------------------------

/// Queued responses for each [`FakeFsPort`] method.
#[derive(Debug, Default)]
pub struct FsScripts {
    /// `read`.
    pub read: Script<Vec<u8>>,
    /// `write`.
    pub write: Script<()>,
    /// `list`.
    pub list: Script<Vec<DirEntry>>,
}

/// One call made on a [`FakeFsPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsCall {
    /// `read(path)`.
    Read(PathBuf),
    /// `write(path, contents)`.
    Write(PathBuf, Vec<u8>),
    /// `list(path)`.
    List(PathBuf),
    /// `watch(path)`.
    Watch(PathBuf),
}

#[derive(Default)]
struct FsData {
    files: BTreeMap<PathBuf, Vec<u8>>,
    dirs: BTreeSet<PathBuf>,
    watchers: Vec<(PathBuf, mpsc::UnboundedSender<FsEvent>)>,
}

impl FsData {
    fn is_dir(&self, path: &Path) -> bool {
        self.dirs.contains(path) || self.files.keys().any(|f| f != path && f.starts_with(path))
    }

    fn emit(&mut self, event: &FsEvent) {
        self.watchers.retain(|(root, tx)| {
            !event.path.starts_with(root) || tx.unbounded_send(event.clone()).is_ok()
        });
    }
}

/// Fake `FsPort`: an in-memory file tree; nothing touches the disk and `watch` never
/// starts a watcher thread.
///
/// `write` creates or replaces a file (parents are implied) and emits `Created` or
/// `Modified` to every `watch` stream whose path is an ancestor; the test can simulate
/// outside changes with [`insert`](Self::insert), [`remove`](Self::remove) and
/// [`emit`](Self::emit). `list` returns direct children (files, and implied or
/// [`with_dir`](Self::with_dir) directories) ordered by path; a missing path is
/// `NotFound`. A scripted response replaces the in-memory behaviour for that call.
#[derive(Default)]
pub struct FakeFsPort {
    script: FsScripts,
    calls: CallLog<FsCall>,
    data: Mutex<FsData>,
}

fake_plumbing!(FakeFsPort, FsScripts, FsCall);

impl std::fmt::Debug for FakeFsPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let data = self.data.lock();
        f.debug_struct("FakeFsPort")
            .field("files", &data.files.keys().collect::<Vec<_>>())
            .field("watchers", &data.watchers.len())
            .finish_non_exhaustive()
    }
}

impl FakeFsPort {
    /// An empty file tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file (without emitting an event).
    #[must_use]
    pub fn with_file(self, path: impl Into<PathBuf>, contents: impl Into<Vec<u8>>) -> Self {
        self.data.lock().files.insert(path.into(), contents.into());
        self
    }

    /// Adds an (empty) directory.
    #[must_use]
    pub fn with_dir(self, path: impl Into<PathBuf>) -> Self {
        self.data.lock().dirs.insert(path.into());
        self
    }

    /// Creates or replaces a file as an outside process would, emitting the event.
    pub fn insert(&self, path: impl Into<PathBuf>, contents: impl Into<Vec<u8>>) {
        let path = path.into();
        let mut data = self.data.lock();
        let kind = if data.files.insert(path.clone(), contents.into()).is_some() {
            FsEventKind::Modified
        } else {
            FsEventKind::Created
        };
        data.emit(&FsEvent { path, kind });
    }

    /// Removes a file as an outside process would, emitting `Removed`. Returns whether it
    /// existed.
    pub fn remove(&self, path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();
        let mut data = self.data.lock();
        let existed = data.files.remove(path).is_some();
        if existed {
            data.emit(&FsEvent {
                path: path.to_owned(),
                kind: FsEventKind::Removed,
            });
        }
        existed
    }

    /// Sends `event` to every matching `watch` stream, without touching the tree.
    pub fn emit(&self, event: FsEvent) {
        self.data.lock().emit(&event);
    }

    /// The current content of `path`, without recording a call.
    pub fn file(&self, path: impl AsRef<Path>) -> Option<Vec<u8>> {
        self.data.lock().files.get(path.as_ref()).cloned()
    }

    /// Number of `watch` streams still alive.
    pub fn watcher_count(&self) -> usize {
        let mut data = self.data.lock();
        data.watchers.retain(|(_, tx)| !tx.is_closed());
        data.watchers.len()
    }
}

#[async_trait]
impl FsPort for FakeFsPort {
    async fn read(&self, path: &Path) -> OxiResult<Vec<u8>> {
        self.calls.record(FsCall::Read(path.to_owned()));
        self.script.read.next_or_else(|| {
            self.data
                .lock()
                .files
                .get(path)
                .cloned()
                .ok_or_else(|| OxiError::not_found(format!("{} not found", path.display())))
        })
    }

    async fn write(&self, path: &Path, contents: &[u8]) -> OxiResult<()> {
        self.calls
            .record(FsCall::Write(path.to_owned(), contents.to_vec()));
        self.script.write.next_or_else(|| {
            self.insert(path, contents);
            Ok(())
        })
    }

    async fn list(&self, path: &Path) -> OxiResult<Vec<DirEntry>> {
        self.calls.record(FsCall::List(path.to_owned()));
        self.script.list.next_or_else(|| {
            let data = self.data.lock();
            if !data.is_dir(path) {
                return Err(OxiError::not_found(format!(
                    "{} is not a directory",
                    path.display()
                )));
            }
            let mut entries: BTreeMap<PathBuf, DirEntry> = BTreeMap::new();
            let children = data
                .files
                .keys()
                .chain(data.dirs.iter())
                .filter_map(|p| p.strip_prefix(path).ok())
                .filter_map(|rest| rest.components().next())
                .map(|first| path.join(first));
            for child in children {
                let entry = match data.files.get(&child) {
                    Some(bytes) => DirEntry {
                        path: child.clone(),
                        kind: EntryKind::File,
                        size: Some(bytes.len() as u64),
                    },
                    None => DirEntry {
                        path: child.clone(),
                        kind: EntryKind::Dir,
                        size: None,
                    },
                };
                entries.insert(child, entry);
            }
            Ok(entries.into_values().collect())
        })
    }

    fn watch(&self, path: &Path) -> BoxStream<'static, FsEvent> {
        self.calls.record(FsCall::Watch(path.to_owned()));
        let (tx, rx) = mpsc::unbounded();
        self.data.lock().watchers.push((path.to_owned(), tx));
        rx.boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use futures::executor::block_on;
    use jiff::Timestamp;
    use oxikube_domain::ErrorKind;
    use oxikube_domain::audit::{AuditOutcome, Initiator};
    use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
    use oxikube_ports::secrets::ExposeSecret;
    use serde_json::json;

    fn key(k: &str) -> StateKey {
        StateKey::new(k).unwrap()
    }

    fn record(cluster: &ClusterId, cmd: &str, secs: i64) -> AuditRecord {
        AuditRecord::new(
            Timestamp::from_second(secs).unwrap(),
            "me",
            Initiator::Ui,
            cmd,
            ResourceRef::namespaced(cluster.clone(), Gvk::new("", "v1", "Pod"), "demo", "web"),
            false,
            AuditOutcome::Succeeded,
        )
    }

    #[test]
    fn state_kv_and_tables_round_trip_and_scripts_errors() {
        let fake = FakeStatePort::new();
        block_on(fake.kv_set(&key("ui.theme"), json!("dark"))).unwrap();
        block_on(fake.kv_set(&key("ui.font"), json!(14))).unwrap();
        block_on(fake.kv_set(&key("window.width"), json!(800))).unwrap();
        assert_eq!(
            block_on(fake.kv_get(&key("ui.theme"))).unwrap(),
            Some(json!("dark"))
        );
        let ui = block_on(fake.kv_list("ui.")).unwrap();
        assert_eq!(
            ui,
            vec![
                (key("ui.font"), json!(14)),
                (key("ui.theme"), json!("dark"))
            ]
        );
        assert!(block_on(fake.kv_delete(&key("ui.font"))).unwrap());
        assert!(!block_on(fake.kv_delete(&key("ui.font"))).unwrap());

        let t = StateTable::new("recent").unwrap();
        for k in ["c", "a", "b"] {
            block_on(fake.table_put(&t, &key(k), json!(k))).unwrap();
        }
        let rows = block_on(fake.table_list(&t, Some(2))).unwrap();
        assert_eq!(rows, vec![(key("a"), json!("a")), (key("b"), json!("b"))]);
        assert_eq!(
            block_on(fake.table_get(&t, &key("c"))).unwrap(),
            Some(json!("c"))
        );
        assert!(block_on(fake.table_delete(&t, &key("c"))).unwrap());

        fake.script()
            .kv_get
            .push_ok(Some(json!(1)))
            .push_err(OxiError::internal("db locked"));
        assert_eq!(block_on(fake.kv_get(&key("x"))).unwrap(), Some(json!(1)));
        assert_eq!(
            block_on(fake.kv_get(&key("x"))).unwrap_err().kind(),
            ErrorKind::Internal
        );
        assert_eq!(
            fake.recorded_calls()[0],
            StateCall::KvSet(key("ui.theme"), json!("dark"))
        );
    }

    #[test]
    fn state_audit_is_append_only_and_queried_newest_first() {
        let fake = FakeStatePort::new();
        let a = ClusterId::new("/k", &ContextName::new("a"));
        let b = ClusterId::new("/k", &ContextName::new("b"));
        block_on(fake.append_audit(&[record(&a, "pod.delete", 10), record(&b, "pod.delete", 20)]))
            .unwrap();
        block_on(fake.append_audit(&[record(&a, "deploy.scale", 30)])).unwrap();
        assert_eq!(fake.audit_log().len(), 3);

        let all = block_on(fake.query_audit(&AuditQuery::default())).unwrap();
        let secs: Vec<i64> = all.iter().map(|r| r.ts.as_second()).collect();
        assert_eq!(secs, vec![30, 20, 10]);
        let only_a = AuditQuery {
            cluster: Some(a),
            since: Some(Timestamp::from_second(10).unwrap()),
            until: Some(Timestamp::from_second(30).unwrap()),
            ..AuditQuery::default()
        };
        let hits = block_on(fake.query_audit(&only_a)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(&*hits[0].cmd, "pod.delete");
        let limited = AuditQuery {
            cmd: Some("pod.delete".into()),
            limit: 1,
            ..AuditQuery::default()
        };
        assert_eq!(
            block_on(fake.query_audit(&limited)).unwrap()[0]
                .ts
                .as_second(),
            20
        );
    }

    #[test]
    fn secrets_live_in_memory_and_are_never_recorded() {
        let fake = FakeSecretStorePort::new();
        let k = SecretKey::new("oxikube", "token-kind-a").unwrap();
        block_on(fake.set(&k, SecretString::from("dummy-token"))).unwrap();
        let got = block_on(fake.get(&k)).unwrap().unwrap();
        assert_eq!(got.expose_secret(), "dummy-token");
        assert_eq!(fake.keys(), vec![k.clone()]);
        assert!(!format!("{fake:?} {:?}", fake.recorded_calls()).contains("dummy-token"));
        fake.script()
            .get
            .push_err(OxiError::auth("keychain locked", true));
        assert_eq!(block_on(fake.get(&k)).unwrap_err().kind(), ErrorKind::Auth);
        assert!(block_on(fake.delete(&k)).unwrap());
        assert!(fake.peek(&k).is_none());
        assert_eq!(
            fake.recorded_calls(),
            vec![
                SecretCall::Set(k.clone()),
                SecretCall::Get(k.clone()),
                SecretCall::Get(k.clone()),
                SecretCall::Delete(k),
            ]
        );
    }

    #[test]
    fn fs_reads_writes_lists_and_watches_in_memory() {
        let fake = FakeFsPort::new()
            .with_file("/cfg/a.yaml", "a: 1")
            .with_file("/cfg/sub/b.yaml", "b")
            .with_dir("/cfg/empty");
        assert_eq!(
            block_on(fake.read(Path::new("/cfg/a.yaml"))).unwrap(),
            b"a: 1"
        );
        assert_eq!(
            block_on(fake.read(Path::new("/nope"))).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        let entries = block_on(fake.list(Path::new("/cfg"))).unwrap();
        let listed: Vec<_> = entries.iter().map(|e| (e.path.clone(), e.kind)).collect();
        assert_eq!(
            listed,
            vec![
                (PathBuf::from("/cfg/a.yaml"), EntryKind::File),
                (PathBuf::from("/cfg/empty"), EntryKind::Dir),
                (PathBuf::from("/cfg/sub"), EntryKind::Dir),
            ]
        );
        assert!(
            block_on(fake.list(Path::new("/cfg/empty")))
                .unwrap()
                .is_empty()
        );
        assert!(block_on(fake.list(Path::new("/missing"))).is_err());

        let mut events = fake.watch(Path::new("/cfg"));
        let mut other = fake.watch(Path::new("/elsewhere"));
        block_on(fake.write(Path::new("/cfg/a.yaml"), b"a: 2")).unwrap();
        block_on(fake.write(Path::new("/cfg/new.yaml"), b"n")).unwrap();
        assert!(fake.remove("/cfg/new.yaml"));
        let kinds: Vec<_> = (0..3)
            .map(|_| block_on(events.next()).unwrap())
            .map(|e| (e.path, e.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (PathBuf::from("/cfg/a.yaml"), FsEventKind::Modified),
                (PathBuf::from("/cfg/new.yaml"), FsEventKind::Created),
                (PathBuf::from("/cfg/new.yaml"), FsEventKind::Removed),
            ]
        );
        assert!(other.next().now_or_never().is_none());
        assert_eq!(fake.file("/cfg/a.yaml"), Some(b"a: 2".to_vec()));

        fake.script()
            .write
            .push_err(OxiError::forbidden("read-only fs"));
        assert!(block_on(fake.write(Path::new("/cfg/a.yaml"), b"x")).is_err());
        assert_eq!(fake.file("/cfg/a.yaml"), Some(b"a: 2".to_vec()));
        drop(events);
        drop(other);
        assert_eq!(fake.watcher_count(), 0);
        assert!(matches!(fake.recorded_calls()[0], FsCall::Read(_)));
    }
}
