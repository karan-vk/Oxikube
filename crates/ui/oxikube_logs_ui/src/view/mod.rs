//! [`LogView`] (E08-S02): the log of one pod's container as a workspace tab.
//!
//! The view is a GPUI entity that is also a workspace [`Item`](oxikube_workspace::Item) (title
//! `pod/container`, draggable between panes like every tab). It holds one
//! [`LogSession`](oxikube_app::logs::LogSession) of the app's `LogService`: the service's task
//! (abort-on-drop, owned by the session) reads the stream into the bounded ring buffer, and the
//! view polls the session's batched deltas once per wake, applies them in one update and redraws
//! through `notify_coalesced`. Changing what is read (range, container, previous instance) drops
//! that session and opens a new one.
//!
//! | File | Holds |
//! |---|---|
//! | `options` | [`ViewOptions`]: range (tail / head / since), container, previous, wrap, timestamps; the port's request |
//! | `window` | [`LineWindow`] (the rows: truncated marker, lines by seq, state row) and [`Follow`] (autoscroll and its "N new lines" count, by seq) |
//! | `text` | what a row says: timestamp, level colour, marker and state words |
//! | `containers` | the container selector's list (init, sidecar, regular, ephemeral) from the pod spec |
//! | `stream` | opening and reopening the session, the delta pump |
//! | `scroll` | autoscroll, pausing on a scroll up, the anchor line across the wrap toggle |
//! | `controls` | the view's operations (what the commands do) and the requests that dispatch them |
//! | `selection` | [`Selection`] (click, shift-click, drag, by seq) and [`Marks`] (k9s `m`, the gutter bar), the pointer handlers |
//! | `pick`, `copy` | which lines an action takes (on screen, the buffer, the filter) and `logs::Copy` (cap 5 MB) |
//! | `save`, `clear`, `notice` | `logs::Save` (dialog, panel, streamed write), `logs::Clear`, the toasts of local actions |
//! | `actions` | the `log_view::*` key actions of the `LogView` key context |
//! | `render`, `toolbar`, `rows` | drawing: toolbar, virtualised rows (`uniform_list` unwrapped, `list` wrapped), the pill |
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
mod clear;
mod containers;
mod controls;
mod copy;
mod item;
mod notice;
mod options;
mod pick;
mod render;
mod rows;
mod save;
mod scroll;
mod selection;
mod stream;
mod text;
mod toolbar;
mod window;

#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, FocusHandle, ListAlignment, ListState, Task,
    UniformListScrollHandle, WeakEntity, px,
};
use oxikube_app::ClusterSessionManager;
use oxikube_app::logs::export::LineFilter;
use oxikube_app::logs::{LogService, LogSession};
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::FsPort;
use oxikube_workspace::{CommandDispatcher, Workspace};

pub use actions::{
    Clear, ClearSelection, Copy, Head, Mark, SaveAll, SaveVisible, Since1h, Since1m, Since5m,
    Since15m, Since30m, Tail, ToggleAutoscroll, ToggleFullscreen, TogglePrevious, ToggleTimestamps,
    ToggleWrap,
};
pub use containers::{ContainerChoice, choices_of, default_container};
pub use copy::COPY_LIMIT_BYTES;
pub use item::item_key;
pub use options::{HEAD_LIMIT_BYTES, OpenLogs, TAIL_LINES, ViewOptions};
pub use selection::{Marks, Selection};
pub use text::{Level, group, level_of, lines_of};
pub use window::{Follow, LineWindow, Row, RowChange};

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
    pub(crate) follow: Follow,
    /// The selected lines, by seq (click, shift-click, drag).
    pub(crate) selection: Selection,
    /// The marked lines, by seq.
    pub(crate) marks: Marks,
    /// What a copy or a save keeps (the search installs its matcher); `None` keeps every line.
    pub(crate) filter: Option<LineFilter>,
    /// The running save (dropping it stops the write); replaced by the next one.
    pub(crate) save_task: Option<Task<()>>,
    /// The unwrapped list's scroll position.
    pub(crate) scroll: UniformListScrollHandle,
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
            ..ViewOptions::default()
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
        let mut view = Self {
            target,
            deps,
            options,
            containers: Vec::new(),
            session: None,
            started: false,
            awaiting_pod: false,
            window: LineWindow::new(),
            follow: Follow::default(),
            selection: Selection::default(),
            marks: Marks::default(),
            filter: None,
            save_task: None,
            scroll: UniformListScrollHandle::new(),
            list: ListState::new(0, ListAlignment::Top, px(400.)),
            focus: cx.focus_handle(),
            workspace: None,
            rows_built: 0,
            pump: None,
            pod_task: None,
        };
        if view.options.container.is_some() {
            view.open_stream(cx);
        }
        view.load_pod(cx);
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
