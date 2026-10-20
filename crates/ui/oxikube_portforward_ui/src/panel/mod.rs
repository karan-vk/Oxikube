//! [`PortForwardPanel`]: the port-forward manager as a bottom-dock panel of the window, listing
//! the forwards of every cluster.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | the panel, its deps, the `Panel` impl, installing it in a workspace |
//! | `live` | the subscription to the manager's changes and the 1 Hz sampler of uptime and traffic |
//! | `delegate` | the table's data source over the [`PanelModel`](crate::PanelModel) |
//! | `keys` | what the keys, clicks and buttons do: select, and dispatch the row actions |
//! | `render` | the toolbar, the table and the status line |
//! | `actions` | the key actions of the `PortForwards` context |
//!
//! # Data, and what it costs
//!
//! The panel owns no forward: it reads the manager through a [`ForwardSource`].
//! A state, address or list change wakes it (the manager coalesces them); the traffic counters and
//! the uptime are sampled on a one second timer, never per byte. Each wake builds the rows off the
//! manager's list (a lock and a clone of a few dozen entries) and redraws only when a cell would
//! show something different, at most once per frame
//! ([`notify_coalesced`](oxikube_runtime::notify_coalesced)). The table is virtualised, so 50
//! forwards cost the visible rows.

mod actions;
mod delegate;
mod keys;
mod live;
mod render;

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Action, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Pixels,
    SharedString, Subscription, Task, Window, px,
};
use oxikube_app::ClusterSessionManager;
use oxikube_ports::ClockPort;
use oxikube_ui::IconName;
use oxikube_ui::input::InputState;
use oxikube_ui::table::{TableHandle, TableOptions};
use oxikube_workspace::panel::{DockPosition, Panel, PanelEvent};
use oxikube_workspace::{CommandDispatcher, Workspace};

pub use actions::{
    Close, CopyUrlSelected, FocusFilter, FocusTable, OpenSelected, RemoveSelected, RestartSelected,
    SelectFirst, SelectLast, SelectNext, SelectPrevious, StopSelected, TogglePanel,
};
pub use delegate::PanelDelegate;
pub use live::SAMPLE_INTERVAL;

use crate::source::ForwardSource;

/// The panel's name in saved layouts. Once chosen, never change it.
pub const PANEL_NAME: &str = "PortForwardsPanel";

/// What the panel is built over.
#[derive(Clone)]
pub struct PortForwardPanelDeps {
    /// The manager (or a fake).
    pub source: Arc<dyn ForwardSource>,
    /// The sessions: they name the clusters by their context.
    pub sessions: ClusterSessionManager,
    /// Where the panel sends its commands (the bus).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// The clock the uptime is measured against.
    pub clock: Arc<dyn ClockPort>,
}

/// The Port Forwards panel. See the [module docs](self).
pub struct PortForwardPanel {
    pub(crate) deps: PortForwardPanelDeps,
    pub(crate) focus: FocusHandle,
    pub(crate) table: TableHandle<PanelDelegate>,
    pub(crate) filter: Entity<InputState>,
    /// Whether the filter field has the focus (bare keys are text then).
    pub(crate) editing: bool,
    /// Whether the panel is the displayed one of its dock group (the sampler idles otherwise).
    pub(crate) active: bool,
    pub(crate) tasks: Vec<Task<()>>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PanelEvent> for PortForwardPanel {}

impl Focusable for PortForwardPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PortForwardPanel {
    /// A panel over `deps`, with the forwards read at once and kept current.
    pub fn new(deps: PortForwardPanelDeps, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let options = TableOptions {
            sortable: false,
            resizable_columns: true,
            movable_columns: false,
            loop_selection: false,
            select_rows: false,
        };
        let table = TableHandle::with_options(PanelDelegate::new(), options, window, cx);
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter forwards"));
        let mut this = Self {
            deps,
            focus: cx.focus_handle(),
            table,
            filter,
            editing: false,
            // Until the workspace displays it: a hidden panel samples nothing.
            active: false,
            tasks: Vec::new(),
            _subscriptions: Vec::new(),
        };
        this.wire(window, cx);
        this
    }

    /// The model: the rows, the filters and the selection.
    pub fn model<'a>(&self, cx: &'a App) -> &'a crate::PanelModel {
        &self.table.delegate(cx).model
    }
}

impl Panel for PortForwardPanel {
    fn persistent_name() -> &'static str {
        PANEL_NAME
    }

    fn panel_key() -> &'static str {
        "port_forwards_panel"
    }

    fn position(&self, _: &Window, _: &App) -> DockPosition {
        DockPosition::Bottom
    }

    fn default_size(&self, _: &Window, _: &App) -> Pixels {
        px(280.)
    }

    fn min_size(&self, _: &Window, _: &App) -> Option<Pixels> {
        Some(px(140.))
    }

    fn icon(&self, _: &Window, _: &App) -> Option<IconName> {
        Some(IconName::Network)
    }

    fn icon_tooltip(&self, _: &Window, _: &App) -> Option<SharedString> {
        Some("Port Forwards".into())
    }

    fn title(&self, cx: &App) -> SharedString {
        match self.model(cx).total() {
            0 => "Port Forwards".into(),
            n => format!("Port Forwards ({n})").into(),
        }
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(TogglePanel)
    }

    fn activation_priority(&self) -> u32 {
        50
    }

    fn set_active(&mut self, active: bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.active != active {
            self.active = active;
            if active {
                self.refresh(cx);
            }
        }
    }
}

/// Adds the panel to `workspace`'s bottom dock, closed until `portforward::TogglePanel` (`:pf`, the
/// table's `f`, the palette, the key). Returns the panel.
pub fn install(
    workspace: &Entity<Workspace>,
    deps: PortForwardPanelDeps,
    window: &mut Window,
    cx: &mut App,
) -> Entity<PortForwardPanel> {
    let panel = cx.new(|cx| PortForwardPanel::new(deps, window, cx));
    workspace.update(cx, |ws, cx| {
        let had_dock = ws.dock(DockPosition::Bottom, cx).is_some();
        ws.add_panel(panel.clone(), window, cx);
        // Adding to a dock that did not exist opens it: the panel starts hidden.
        if !had_dock
            && ws
                .dock(DockPosition::Bottom, cx)
                .is_some_and(|d| d.is_open())
        {
            ws.toggle_dock(DockPosition::Bottom, window, cx);
        }
    });
    panel
}
