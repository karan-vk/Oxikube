//! The manifest editor against the live object (E10-S07): a watch on the one object the buffer
//! is about, the "changed on server" banner, and the diff in the editor's bottom panel.
//!
//! | Module | What |
//! |---|---|
//! | `watch` | the producer on the Kubernetes runtime: a `metadata.name` field-selector watch of the object, reduced to its latest state ([`Watched`]) |
//! | `actions` | the commands' effects (show the diff, reload, merge, keep mine, a side per conflict) and the diff computed off the UI thread for the panel |
//! | `link` | the editor's side: binding to an object, the watch's results built into [`ObjectVersion`]s off the UI thread, [`LiveSync`] decisions, the diff recomputed off the UI thread, the commands |
//! | `opened` | an editor opened from an object (E10-S06) follows it: its rendering is the base |
//! | `banner` | the banner (Merge / Reload / Keep mine) |
//! | `conflicts` | the merge's conflicts with a side to pick each, virtualised (`uniform_list`) |
//! | `panel` | the bottom panel: the diff side by side or inline, virtualised (`uniform_list`) |
//!
//! The watch is the editor's own: started when the editor is bound to an object (loaded from it,
//! or asked for a diff or a reload of the object its manifest names), dropped with the editor
//! (closing the tab aborts it: `spawn_kube` is abort-on-drop) or when it is bound to another
//! object. It goes through the session's resource port, whose feeds count toward the cluster's
//! watch budget. Watch events are coalesced: only the latest state is built and applied, and the
//! view is notified at most once per frame.
//!
//! [`ObjectVersion`]: crate::diff::ObjectVersion
//! [`LiveSync`]: crate::diff::LiveSync

mod actions;
mod banner;
mod conflicts;
mod link;
mod opened;
mod panel;
mod watch;

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, SharedString, Task, UniformListScrollHandle};
use oxikube_app::ClusterSessionManager;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_ports::ResourceReader;

use crate::diff::{LiveSync, ManifestDiff, Renderer, default_renderer};

pub(super) use panel::{ROW_HEIGHT, inline_rows, summary};
pub(super) use watch::Watched;

/// How long the buffer must rest before the open diff is computed again.
pub const DIFF_DEBOUNCE: Duration = Duration::from_millis(150);

/// Where an editor reads its live object: the cluster's resource port.
pub trait LiveSource: 'static {
    /// The cluster the objects are in.
    fn cluster(&self) -> &ClusterId;

    /// The port to read and watch with, `None` while the cluster is not connected.
    fn reader(&self, cx: &App) -> Option<Arc<dyn ResourceReader>>;
}

/// The app's [`LiveSource`]: the session of `cluster`, asked per watch so an editor does not keep
/// a disconnected session's ports alive.
#[derive(Clone)]
pub struct SessionLive {
    sessions: ClusterSessionManager,
    cluster: ClusterId,
}

impl SessionLive {
    /// The objects of `cluster`'s session.
    pub fn new(sessions: ClusterSessionManager, cluster: ClusterId) -> Self {
        Self { sessions, cluster }
    }
}

impl LiveSource for SessionLive {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    fn reader(&self, _: &App) -> Option<Arc<dyn ResourceReader>> {
        self.sessions.get(&self.cluster)?.resources()
    }
}

/// How the diff is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffLayout {
    /// The server's lines on the left, the buffer's on the right.
    #[default]
    SideBySide,
    /// One column, removed lines above added ones.
    Inline,
}

/// What the bottom panel shows.
#[derive(Clone, Debug, Default)]
pub(super) enum PanelBody {
    /// Waiting for the object or the diff.
    #[default]
    Loading,
    /// The diff of the buffer against the live object.
    Diff(Arc<ManifestDiff>),
    /// Why there is no diff ("Not connected", "The manifest is not valid YAML").
    Message(SharedString),
}

/// The bottom panel's state.
#[derive(Default)]
pub(super) struct DiffPanel {
    pub(super) open: bool,
    pub(super) layout: DiffLayout,
    pub(super) body: PanelBody,
    pub(super) scroll: UniformListScrollHandle,
    /// The merge's conflict list.
    pub(super) conflict_scroll: UniformListScrollHandle,
    /// The diff being computed; replacing it cancels it.
    pub(super) task: Option<Task<()>>,
}

/// An editor's link to its live object: the source, the object, the watch, the sync state and
/// the diff panel.
pub(crate) struct LiveLink {
    pub(super) source: Option<Rc<dyn LiveSource>>,
    pub(super) render: Renderer,
    /// The object followed, once known.
    pub(super) target: Option<ResourceRef>,
    pub(super) sync: LiveSync,
    /// Bumped by every live change, so a diff computed against an older object is dropped.
    pub(super) generation: u64,
    /// The watch on the Kubernetes runtime and the task applying its results.
    pub(super) watch: Option<Task<()>>,
    /// Whether that watch is still open (it ends with its stream).
    pub(super) watching: bool,
    /// The latest watch result not built yet, and whether a build is running.
    pub(super) pending: Option<Watched>,
    pub(super) building: bool,
    /// The last error of the watch or of a load, shown in the panel and the banner area.
    pub(super) problem: Option<SharedString>,
    /// A lookup of the object the buffer names; replacing it cancels it.
    pub(super) loading: Option<Task<()>>,
    /// Bumped by every load ([`ManifestEditor::follow_object`]): an older one in flight is
    /// dropped when it finishes.
    ///
    /// [`ManifestEditor::follow_object`]: crate::view::ManifestEditor::follow_object
    pub(super) load_seq: u64,
    /// A merge being computed; replacing it cancels it.
    pub(super) merging: Option<Task<()>>,
    pub(super) panel: DiffPanel,
}

impl LiveLink {
    /// Whether there is a cluster to read the live object from.
    pub(super) fn has_source(&self) -> bool {
        self.source.is_some()
    }

    pub(super) fn new(source: Option<Rc<dyn LiveSource>>) -> Self {
        Self {
            source,
            render: default_renderer(),
            target: None,
            sync: LiveSync::new(),
            generation: 0,
            watch: None,
            watching: false,
            pending: None,
            building: false,
            problem: None,
            loading: None,
            load_seq: 0,
            merging: None,
            panel: DiffPanel::default(),
        }
    }

    /// "Deployment/web", for the banner and the panel's title.
    pub(super) fn target_label(&self) -> Option<String> {
        let target = self.target.as_ref()?;
        Some(format!("{}/{}", target.gvk.kind, target.name))
    }
}
