//! [`LogView`] (E08-S02): the log of one pod's container as a workspace tab.
//!
//! The view is a GPUI entity that is also a workspace [`Item`](oxikube_workspace::Item) (title
//! `pod/container`, draggable between panes like every tab). It holds one
//! [`LogSession`] of the app's `LogService`: the service's task
//! (abort-on-drop, owned by the session) reads the stream into the bounded ring buffer, and the
//! view polls the session's batched deltas once per wake, applies them in one update and redraws
//! through `notify_coalesced`. Changing what is read (range, container, previous instance) drops
//! that session and opens a new one.
//!
//! | File | Holds |
//! |---|---|
//! | `aggregate` | a workload or Service as one merged log (E08-S04): pod colours and gutters, the banner, switching sources off |
//! | `options` | [`ViewOptions`]: range (tail / head / since), container, previous, wrap, timestamps; the port's request |
//! | `window` | [`LineWindow`] (the rows: truncated marker, lines by seq or the search's matches, state row) |
//! | `autoscroll` | [`Follow`] (autoscroll and its "N new lines" count, by seq) |
//! | `text` | what a row says: timestamp, level colour, marker and state words |
//! | `containers` | the container selector's list (init, sidecar, regular, ephemeral) from the pod spec |
//! | `stream` | opening and reopening the session, the delta pump |
//! | `settings` | the `logs` settings of the view's cluster: its first options, and `wrap` / `timestamps` / `json_auto_detect` applied live |
//! | `snap` | the slack above the unwrapped rows that keeps every row on screen whole |
//! | `scroll` | autoscroll, pausing on a scroll up, the anchor line across the wrap toggle |
//! | `controls` | the view's operations (what the commands do) and the requests that dispatch them |
//! | `selection`, `chrome` | [`Selection`] (click, shift-click, drag, by seq) and [`Marks`] (k9s `m`), the pointer handlers; the gutter bar and selection colour a row carries |
//! | `agent` | `logs::SendToAgent`: the selection (else the lines on screen) as agent context, with its source |
//! | `pick`, `copy` | which lines an action takes (on screen, the buffer, the filter) and `logs::Copy` (cap 5 MB) |
//! | `save`, `clear`, `notice` | `logs::Save` (dialog, panel, streamed write), `logs::Clear`, the toasts of local actions |
//! | `tail` | `logs::TailInTerminal` (E08-S08): `kubectl logs -f` for what the view shows, in a terminal tab; the toolbar offers it only when kubectl is installed |
//! | `recovery` | after the stream stopped (E08-S07): `logs::FollowReplacement` (switch to the pod that replaced a gone one), `logs::Reconnect`, the strip offering them |
//! | `actions` | the `log_view::*` key actions of the `LogView` key context |
//! | `render`, `rows` | drawing: the frame, virtualised rows (`uniform_list` unwrapped, `list` wrapped), the pill |
//! | `toolbar` | the one-row toolbar (E08-U556): breadcrumb, container picker, range dropdown, Search / Previous / Wrap / Autoscroll, the "..." menu; the crash-loop hint and the level chips |
//! | `json`, `columns`, `filter`, `detail` | JSON mode (E08-S05): the parsed columns of a structured line and their caches, the row they draw, the level chips and the filtered row index, the expanded line's pane |
//! | `item` | the workspace `Item`, focus and key context |
//!
//! # Rendering and performance
//!
//! Lines stay in the session's ring buffer; a frame reads only the rows on screen, by seq, under
//! one short lock. Unwrapped, every row has one height and the list is a `uniform_list`; wrapped,
//! rows have their own heights and the list is a `list` over a `ListState`, kept in step with
//! the deltas by splices (only the rows on screen are measured). GPUI's line layout cache keeps
//! the shaped text of the rows drawn in the previous frame, keyed by text, font, size and colour
//! runs, so a streaming view shapes only the rows that scroll in; a theme or zoom change is a new
//! key (PERFORMANCE rule 5). Unwrapped rows draw at most [`NOWRAP_CHARS`] bytes of a line.

mod actions;
mod agent;
mod aggregate;
mod autoscroll;
mod chrome;
mod clear;
mod columns;
mod containers;
mod controls;
mod copy;
mod detail;
mod filter;
mod highlight;
mod item;
mod json;
mod notice;
mod options;
mod pick;
mod recovery;
mod render;
mod rows;
mod save;
mod scroll;
mod selection;
mod settings;
mod snap;
mod stream;
mod tail;
pub(crate) mod text;
mod toolbar;
mod window;

#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, ListAlignment, ListState, Subscription, Task,
    UniformListScrollHandle, WeakEntity, px,
};
use oxikube_app::ClusterSessionManager;
use oxikube_app::context::PendingContext;
use oxikube_app::logs::export::LineFilter;
use oxikube_app::logs::kubectl::Kubectl;
use oxikube_app::logs::{AggregateSpec, LevelFilter, LogService, LogSession};
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::FsPort;
use oxikube_settings::Settings as _;
use oxikube_terminal::view::TerminalViewSink;
use oxikube_workspace::{CommandDispatcher, Workspace};

use crate::LogsSettings;
use crate::search::Search;

pub use actions::{
    Clear, ClearSelection, CloseSearch, Copy, Find, FollowReplacement, Head, Mark, NextMatch,
    PreviousMatch, Reconnect, SaveAll, SaveVisible, SendToAgent, Since1h, Since1m, Since5m,
    Since15m, Since30m, Tail, TailInTerminal, ToggleAutoscroll, ToggleCase, ToggleFilterMode,
    ToggleFullscreen, ToggleInverse, ToggleJsonMode, TogglePrevious, ToggleTimestamps, ToggleWrap,
};
pub use aggregate::{
    AggregateState, BANNER_LINES, BANNER_SECONDS, Banner, MAX_GUTTER, Prefix, SourceChoice,
    SourceLabels, colour_index, short_names,
};
pub use autoscroll::Follow;
pub use containers::{ContainerChoice, choices_of, default_container};
pub use copy::COPY_LIMIT_BYTES;
pub use item::item_key;
pub use json::JsonColumns;
pub use options::{HEAD_LIMIT_BYTES, OpenLogs, TAIL_LINES, ViewOptions};
pub use recovery::Recovery;
pub use selection::{Marks, Selection};
pub use text::{Level, group, level_of, lines_of};
pub use window::{LineWindow, Row, RowChange};

/// Bytes of a line an unwrapped row draws: more than any screen is wide, and a 16 KiB line costs
/// no more to shape than a short one.
pub const NOWRAP_CHARS: usize = 1_024;

/// What a [`LogView`] is built over. Cheap to clone.
#[derive(Clone)]
pub struct LogViewDeps {
    /// The app's one log service: the sessions, their ring buffers and `logs.buffer_lines`.
    pub service: Arc<LogService>,
    /// The sessions: the cluster's `LogPort` and the reader the pod is read with.
    pub sessions: ClusterSessionManager,
    /// Where the view's commands go (the bus): `logs::*` from the keys and the toolbar.
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The files `logs::Save` writes (a path the user chose, streamed in chunks).
    pub fs: Arc<dyn FsPort>,
    /// Where `logs::SendToAgent` queues the selected lines until the agent panel takes them.
    pub agent: PendingContext,
    /// Whether kubectl is installed (the "Tail in terminal" action is hidden without it); the
    /// binary keeps the answer fresh on a background task.
    pub kubectl: Kubectl,
    /// Where "Tail in terminal" asks for its terminal tab.
    pub terminal: TerminalViewSink,
}

/// The log of one pod's container. See the [module docs](self).
pub struct LogView {
    pub(crate) target: ResourceRef,
    pub(crate) deps: LogViewDeps,
    pub(crate) options: ViewOptions,
    pub(crate) containers: Vec<ContainerChoice>,
    pub(crate) session: Option<LogSession>,
    /// Whether the stream was opened (a view that names no container waits for the pod).
    pub(crate) started: bool,
    /// Whether the view waits for the pod to name its default container before it opens the
    /// stream: a change of what is read until then is kept in the options and opens with them.
    pub(crate) awaiting_pod: bool,
    pub(crate) window: LineWindow,
    /// The level chips (JSON mode filters by them; see [`LogView::levels`]).
    pub(crate) levels: LevelFilter,
    /// Whether the session has delivered a structured (JSON) line: the JSON controls (the toggle
    /// and the level chips) appear with the first, so a plain-text log looks as it always did.
    pub(crate) saw_json: bool,
    /// The line shown in the detail pane (JSON mode), if any.
    pub(crate) expanded: Option<detail::Expanded>,
    /// Columns and pretty text of the JSON lines drawn, by seq (drawing reads it).
    pub(crate) records: RefCell<json::RecordCache>,
    pub(crate) follow: Follow,
    /// The selected lines, by seq (click, shift-click, drag).
    pub(crate) selection: Selection,
    /// The marked lines, by seq.
    pub(crate) marks: Marks,
    /// What a copy or a save keeps (the search installs its matcher); `None` keeps every line.
    pub(crate) filter: Option<LineFilter>,
    /// The save panel being answered (dropping it forgets the question).
    pub(crate) save_prompt: Option<Task<()>>,
    /// The running save; [`stop_save`](LogView::stop_save) ends it and tells the user.
    pub(crate) save_job: Option<save::SaveJob>,
    /// The unwrapped list's scroll position.
    pub(crate) scroll: UniformListScrollHandle,
    /// The empty space above the unwrapped rows that keeps every row whole (see `snap`).
    pub(crate) snap: snap::RowSnap,
    /// The wrapped list's rows and scroll position (kept in step with the window by splices).
    pub(crate) list: ListState,
    pub(crate) focus: FocusHandle,
    /// The workspace the view is a tab of (its cluster tab's), for fullscreen.
    pub(crate) workspace: Option<WeakEntity<Workspace>>,
    /// Rows built in the last frame (the test of "only visible rows are built").
    pub(crate) rows_built: usize,
    /// Polls the session's deltas; replaced (so cancelled) when the session is.
    pub(crate) pump: Option<Task<()>>,
    /// Reads the pod for the container selector.
    pub(crate) pod_task: Option<Task<()>>,
    /// Looks for the pod that replaced this one (`logs::FollowReplacement`).
    pub(crate) replacement_task: Option<Task<()>>,
    /// The `logs` settings as last applied to the options (a change applies the keys that moved).
    pub(crate) settings: LogsSettings,
    /// Applies changes of the `logs` settings; dropped with the view.
    pub(crate) _settings_subscription: Subscription,
    /// The search bar's state (E08-S03).
    pub(crate) search: Search,
    /// Set when the view shows a workload or Service (several pods merged), not one pod.
    pub(crate) aggregate: Option<AggregateState>,
}

impl LogView {
    /// A view of `target`'s log (a pod), reading `container` (the pod's default when `None`).
    /// The pod is read for the container selector; the stream opens at once on the service's
    /// runtime when `container` is named, else once the pod says which container is the default.
    pub fn new(
        target: ResourceRef,
        container: Option<String>,
        deps: LogViewDeps,
        cx: &mut Context<Self>,
    ) -> Self {
        let options = ViewOptions {
            container,
            ..ViewOptions::from_settings(&LogsSettings::resolve(&target.cluster, cx))
        };
        Self::with_options(target, options, deps, cx)
    }

    /// [`LogView::new`] with every option given (what `pod::ViewLogs` asks: previous instance,
    /// tail length, follow). The stream opens once, with these options.
    pub fn with_options(
        target: ResourceRef,
        options: ViewOptions,
        deps: LogViewDeps,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::blank(target, options, deps, cx);
        if view.options.container.is_some() {
            view.open_stream(cx);
        }
        view.load_pod(cx);
        view
    }

    /// A view that reads nothing yet.
    fn blank(
        target: ResourceRef,
        options: ViewOptions,
        deps: LogViewDeps,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = LogsSettings::resolve(&target.cluster, cx);
        Self {
            target,
            deps,
            options,
            containers: Vec::new(),
            session: None,
            started: false,
            awaiting_pod: false,
            window: LineWindow::new(),
            levels: LevelFilter::all(),
            saw_json: false,
            expanded: None,
            records: RefCell::default(),
            follow: Follow::default(),
            selection: Selection::default(),
            marks: Marks::default(),
            filter: None,
            save_prompt: None,
            save_job: None,
            scroll: UniformListScrollHandle::new(),
            snap: snap::RowSnap::default(),
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            focus: cx.focus_handle(),
            workspace: None,
            rows_built: 0,
            pump: None,
            pod_task: None,
            replacement_task: None,
            settings,
            _settings_subscription: LogsSettings::observe_in(cx, Self::settings_changed),
            search: Search::default(),
            aggregate: None,
        }
    }

    /// A view of the logs of every pod `target` (a Deployment, StatefulSet, DaemonSet,
    /// ReplicaSet, Job or Service) selects, merged by server timestamp, one colour per pod. The
    /// options' `container` (when set) reads only the containers of that name and `selector`
    /// narrows the pods. The stream opens at once on the service's runtime.
    ///
    /// # Panics
    ///
    /// When `target` is not such an object: check [`AggregateSpec::of`] first.
    pub fn workload(
        target: ResourceRef,
        options: ViewOptions,
        deps: LogViewDeps,
        cx: &mut Context<Self>,
    ) -> Self {
        let spec = AggregateSpec::of(&target).expect("a workload or Service of a namespace");
        let mut view = Self::blank(target, options, deps, cx);
        view.aggregate = Some(AggregateState::new(spec));
        view.open_stream(cx);
        view
    }

    /// [`LogView::new`] as an entity.
    pub fn build(
        target: ResourceRef,
        container: Option<String>,
        deps: LogViewDeps,
        cx: &mut gpui::App,
    ) -> Entity<Self> {
        cx.new(|cx| Self::new(target, container, deps, cx))
    }

    /// What the view was opened on (the pod).
    pub fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// The options in effect.
    pub fn options(&self) -> &ViewOptions {
        &self.options
    }

    /// The rows and the session's state as of the last delta.
    pub fn line_window(&self) -> &LineWindow {
        &self.window
    }

    /// Whether the view follows the newest line.
    pub fn autoscroll(&self) -> bool {
        self.follow.is_on()
    }

    /// The "N new lines" count while autoscroll is paused (0 while following).
    pub fn new_lines(&self) -> u64 {
        self.follow.new_lines(self.window.next_seq())
    }

    /// The session the view reads now (`None` when its cluster is not connected).
    pub fn session(&self) -> Option<&LogSession> {
        self.session.as_ref()
    }

    /// Rows built in the last frame.
    pub fn rows_built(&self) -> usize {
        self.rows_built
    }

    /// Tells the view which workspace it is a tab of (for fullscreen).
    pub fn set_workspace(&mut self, workspace: WeakEntity<Workspace>) {
        self.workspace = Some(workspace);
    }
}
