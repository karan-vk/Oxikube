//! Tests of the Releases page in a workspace window over a fake [`ReleasesBackend`] with static
//! data, and a dispatcher that records commands. (The service's own behaviour is tested in
//! `oxikube_app::helm_releases` over `FakeHelmPort`.)

mod live;
mod model;
mod page;
mod tabs;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::future::BoxFuture;
use futures::stream::LocalBoxStream;
use gpui::{AppContext as _, Entity, Modifiers, Point, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use oxikube_app::helm_releases::{ManifestView, ValuesView};
use oxikube_domain::command::Command;
use oxikube_domain::helm::{
    ChartRef, HelmCapabilities, HelmVersion, Release, ReleaseRef, ReleaseStatus, Revision, Values,
    mask_values,
};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, OpenOptions};
use parking_lot::Mutex;
use tokio::sync::watch;

use super::{
    HelmReleasesView, ObjectRow, ObjectState, ObjectsLease, ReleasesBackend, ReleasesDeps,
};
use crate::{HelmGateState, HelmPage};

pub(super) fn cluster() -> ClusterId {
    ClusterId::new("test", &ContextName::from("kind"))
}

pub(super) fn at(secs: i64) -> Timestamp {
    Timestamp::from_second(secs).expect("in range")
}

pub(super) fn release(ns: &str, name: &str, status: ReleaseStatus, revision: u32) -> Release {
    let mut release = Release::summary(
        ReleaseRef::new(cluster(), ns, name),
        ChartRef::new(name, "1.2.3"),
        revision,
        status,
        at(1_700_000_000 + i64::from(revision)),
    );
    release.app_version = Some("2.0.0".into());
    release
}

pub(super) fn release_ref(name: &str) -> ReleaseRef {
    ReleaseRef::new(cluster(), "apps", name)
}

pub(super) fn object(kind: &str, ns: Option<&str>, name: &str, state: ObjectState) -> ObjectRow {
    let gvk = Gvk::new("", "v1", kind);
    ObjectRow {
        reference: match ns {
            Some(ns) => ResourceRef::namespaced(cluster(), gvk, ns, name),
            None => ResourceRef::cluster_scoped(cluster(), gvk, name),
        },
        source: None,
        state,
    }
}

/// The fixture's shared state: what the fake serves and what it was asked.
#[derive(Default)]
pub(super) struct Data {
    pub releases: Vec<Release>,
    pub history: HashMap<String, Vec<Revision>>,
    pub user_values: HashMap<String, Values>,
    pub all_values: HashMap<String, Values>,
    pub manifest: HashMap<String, String>,
    pub notes: HashMap<String, Option<String>>,
    pub objects: Vec<ObjectRow>,
    pub revealed: HashSet<ReleaseRef>,
    pub fail_list: Option<(oxikube_domain::ErrorKind, String)>,
    /// What was asked, in order: `list`, `values:<name>:<revision>:<all>`, `manifest:<name>`, ...
    pub calls: Vec<String>,
}

pub(super) type Shared = Arc<Mutex<Data>>;

/// A lease whose objects the test changes.
struct FakeLease(Arc<Mutex<Vec<ObjectRow>>>);

impl ObjectsLease for FakeLease {
    fn objects(&self) -> Vec<ObjectRow> {
        self.0.lock().clone()
    }
}

pub(super) struct FakeBackend {
    cluster: ClusterId,
    pub data: Shared,
    /// What the lease reads; the test pushes state changes here.
    pub live_objects: Arc<Mutex<Vec<ObjectRow>>>,
    reveal_tx: watch::Sender<u64>,
    changes: RefCell<Option<mpsc::UnboundedReceiver<()>>>,
}

impl FakeBackend {
    fn view(&self, release: &ReleaseRef, text: Option<Values>, all: bool) -> OxiResult<ValuesView> {
        let _ = all;
        let values = text.ok_or_else(|| OxiError::not_found(format!("{release} has no values")))?;
        let (masked, sensitive) = mask_values(&values);
        let revealed = self.data.lock().revealed.contains(release);
        let shown = if revealed { &values } else { &masked };
        Ok(ValuesView {
            text: Arc::from(shown.to_yaml().expect("yaml")),
            sensitive,
            revealed,
            empty: values.is_empty(),
        })
    }

    /// What `helm::RevealValues` does in the app: grants (or withdraws) and tells the views.
    pub fn grant(&self, release: &ReleaseRef, on: bool) {
        {
            let mut data = self.data.lock();
            if on {
                data.revealed.insert(release.clone());
            } else {
                data.revealed.remove(release);
            }
        }
        self.reveal_tx.send_modify(|n| *n += 1);
    }
}

impl ReleasesBackend for FakeBackend {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    fn list(&self) -> BoxFuture<'static, OxiResult<Vec<Release>>> {
        let data = self.data.clone();
        Box::pin(async move {
            let mut data = data.lock();
            data.calls.push("list".into());
            match data.fail_list.clone() {
                Some((kind, why)) => Err(OxiError::new(kind, why)),
                None => Ok(data.releases.clone()),
            }
        })
    }

    fn watch(&self) -> Option<LocalBoxStream<'static, ()>> {
        self.changes.borrow_mut().take().map(|rx| rx.boxed_local())
    }

    fn history(&self, release: &ReleaseRef) -> BoxFuture<'static, OxiResult<Vec<Revision>>> {
        let (data, name) = (self.data.clone(), release.name.clone());
        Box::pin(async move {
            let mut data = data.lock();
            data.calls.push(format!("history:{name}"));
            Ok(data.history.get(&name).cloned().unwrap_or_default())
        })
    }

    fn values(
        &self,
        release: &ReleaseRef,
        revision: u32,
        all: bool,
    ) -> BoxFuture<'static, OxiResult<ValuesView>> {
        let mut data = self.data.lock();
        data.calls
            .push(format!("values:{}:{revision}:{all}", release.name));
        let found = if all {
            data.all_values.get(&release.name)
        } else {
            data.user_values.get(&release.name)
        }
        .cloned();
        drop(data);
        let result = self.view(release, found, all);
        Box::pin(async move { result })
    }

    fn manifest(
        &self,
        release: &ReleaseRef,
        revision: u32,
    ) -> BoxFuture<'static, OxiResult<ManifestView>> {
        let mut data = self.data.lock();
        data.calls
            .push(format!("manifest:{}:{revision}", release.name));
        let text = data
            .manifest
            .get(&release.name)
            .cloned()
            .unwrap_or_default();
        let revealed = data.revealed.contains(release);
        drop(data);
        let view = ManifestView {
            text: Arc::from(if revealed {
                text
            } else {
                text.replace("c2VjcmV0", "********")
            }),
            masked: !revealed,
        };
        Box::pin(async move { Ok(view) })
    }

    fn notes(
        &self,
        release: &ReleaseRef,
        revision: u32,
    ) -> BoxFuture<'static, OxiResult<Option<Arc<str>>>> {
        let mut data = self.data.lock();
        data.calls
            .push(format!("notes:{}:{revision}", release.name));
        let notes = data.notes.get(&release.name).cloned().flatten();
        drop(data);
        Box::pin(async move { Ok(notes.map(Arc::from)) })
    }

    fn objects(
        &self,
        release: &ReleaseRef,
        revision: u32,
        _: Timestamp,
    ) -> BoxFuture<'static, OxiResult<Box<dyn ObjectsLease>>> {
        self.data
            .lock()
            .calls
            .push(format!("objects:{}:{revision}", release.name));
        let live = self.live_objects.clone();
        Box::pin(async move { Ok(Box::new(FakeLease(live)) as Box<dyn ObjectsLease>) })
    }

    fn reveal_changes(&self) -> watch::Receiver<u64> {
        self.reveal_tx.subscribe()
    }

    fn is_revealed(&self, release: &ReleaseRef) -> bool {
        self.data.lock().revealed.contains(release)
    }

    fn conceal(&self, release: &ReleaseRef) {
        self.grant(release, false);
    }
}

/// Records every command; nothing runs.
#[derive(Clone, Default)]
pub(super) struct TestBus {
    sent: Rc<RefCell<Vec<Command>>>,
}

impl TestBus {
    pub fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }
}

impl CommandDispatcher for TestBus {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command);
    }
}

pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub view: Entity<HelmReleasesView>,
    pub backend: Rc<FakeBackend>,
    pub gate: Entity<HelmGateState>,
    pub bus: TestBus,
    /// Tells the view the release records changed.
    pub changed: mpsc::UnboundedSender<()>,
}

pub(super) fn full() -> HelmCapabilities {
    HelmCapabilities::full(HelmVersion::new(3, 14, 2))
}

impl Harness {
    /// Opens the page over `releases` with a `helm` binary found (or `native` only).
    pub fn open(cx: &mut TestAppContext, data: Data, caps: HelmCapabilities) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
        let live_objects = Arc::new(Mutex::new(data.objects.clone()));
        let (changed, changes) = mpsc::unbounded();
        let backend = Rc::new(FakeBackend {
            cluster: cluster(),
            data: Arc::new(Mutex::new(data)),
            live_objects,
            reveal_tx: watch::channel(0).0,
            changes: RefCell::new(Some(changes)),
        });
        let bus = TestBus::default();
        let mut parts = None;
        vcx.update(|window, cx| {
            let gate = HelmGateState::install(cx);
            gate.update(cx, |gate, cx| {
                gate.set_native_read(true, cx);
                gate.set_cli_capabilities(caps, cx);
            });
            let deps = ReleasesDeps {
                backend: backend.clone(),
                dispatcher: Rc::new(bus.clone()),
                gate: gate.clone(),
            };
            let view = cx.new(|cx| HelmReleasesView::new(deps, window, cx));
            let page = {
                let (gate, view) = (gate.clone(), view.clone());
                cx.new(|cx| HelmPage::with_releases(gate, Rc::new(TestBus::default()), view, cx))
            };
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(page), OpenOptions::default(), window, cx);
            });
            parts = Some((view, gate));
        });
        let (view, gate) = parts.expect("built");
        let mut harness = Self {
            vcx,
            view,
            backend,
            gate,
            bus,
            changed,
        };
        harness.settle();
        harness
    }

    /// Runs what is ready and lets the redraws land.
    pub fn settle(&mut self) {
        for _ in 0..4 {
            self.vcx.run_until_parked();
        }
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&HelmReleasesView, &gpui::App) -> R) -> R {
        let view = self.view.clone();
        self.vcx.read(|cx| f(view.read(cx), cx))
    }

    pub fn names(&mut self) -> Vec<String> {
        self.read(|v, cx| v.table.read(cx, |d| d.shown_names()))
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector).is_some()
    }

    pub fn centre(&mut self, selector: &str) -> Point<gpui::Pixels> {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center()
    }

    pub fn click(&mut self, selector: &str) {
        let at = self.centre(selector);
        self.vcx.simulate_click(at, Modifiers::none());
        self.settle();
    }

    /// Selects row `row` (a click on the row), then lets the reads land.
    pub fn select(&mut self, row: usize) {
        let view = self.view.clone();
        self.vcx
            .update(|_, cx| view.update(cx, |v, cx| v.select_row(row, cx)));
        self.settle();
    }

    pub fn calls(&self) -> Vec<String> {
        self.backend.data.lock().calls.clone()
    }

    pub fn detail<R>(&mut self, f: impl FnOnce(&super::ReleaseDetail) -> R) -> R {
        self.read(|v, _| f(v.detail.as_ref().expect("a detail is open")))
    }
}

/// Every status, one release each, so the table has a row of each tone.
pub(super) fn every_status() -> Vec<Release> {
    ReleaseStatus::ALL
        .into_iter()
        .enumerate()
        .map(|(ix, status)| release("apps", &format!("rel-{ix}-{status}"), status, ix as u32 + 1))
        .collect()
}
