//! `#[gpui::test]`s of the notification centre in a window, over a real `NotificationService`
//! fed through `submit` (the event side is tested in `oxikube_app`), a fake state port and a
//! recording dispatcher that plays the bus: the panel toggled by the command and the key, the
//! unread badge, click-to-navigate, the keys of the panel, the mute toggle and the toast cap.

mod panel;
mod toasts;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::executor::block_on;
use gpui::{Entity, Task, TestAppContext, VisualTestContext};
use oxikube_app::events::{EventService, EventsConfig};
use oxikube_app::notifications::{NotificationInput, NotificationService};
use oxikube_app::{ClusterSessionManager, session::SessionOptions};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_keymap::KeymapOptions;
use oxikube_ports::{ClusterContext, NotificationLevel, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::{NotificationDeps, NotificationPanelSink, NotificationWiring, install, serve};
use crate::cluster_tab::CommandDispatcher;
use crate::panel::DockPosition;
use crate::test_support::open_workspace;
use crate::workspace::Workspace;

pub(super) fn prod() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("prod-eu"))
}

/// Pod `name` in `default` of `prod`.
pub(super) fn pod(name: &str) -> ResourceRef {
    ResourceRef::new(
        prod(),
        Gvk::new("", "v1", "Pod"),
        Some("default".into()),
        name,
    )
}

/// A warning about pod `name` with `reason`.
pub(super) fn warning(name: &str, reason: &str) -> NotificationInput {
    NotificationInput {
        cluster: prod(),
        level: NotificationLevel::Warning,
        title: format!("Pod default/{name}: {reason}"),
        body: format!("{reason} on {name}"),
        target: Some(pod(name)),
        dedupe_key: format!("event/{name}/{reason}").into(),
        group: None,
        occurrences: 1,
    }
}

/// Records every command and plays the bus for the centre's own ones.
#[derive(Clone)]
pub(super) struct Recorder {
    sent: Rc<RefCell<Vec<Command>>>,
    service: NotificationService,
    sink: NotificationPanelSink,
}

impl Recorder {
    pub(super) fn sent(&self) -> Vec<Command> {
        self.sent.borrow().clone()
    }

    pub(super) fn clear(&self) {
        self.sent.borrow_mut().clear();
    }
}

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        match &command {
            Command::NotificationsTogglePanel => {
                self.sink.request();
            }
            Command::NotificationsMarkRead { id } => {
                self.service
                    .mark_read(oxikube_app::notifications::NotificationId(*id));
            }
            Command::NotificationsMarkAllRead => {
                self.service.mark_all_read();
            }
            Command::NotificationsMuteCluster { cluster } => {
                block_on(self.service.mute(cluster)).expect("saved");
            }
            Command::NotificationsUnmuteCluster { cluster } => {
                block_on(self.service.unmute(cluster)).expect("saved");
            }
            _ => {}
        }
        self.sent.borrow_mut().push(command);
    }
}

/// A window with the notification centre installed.
pub(super) struct Fixture {
    pub(super) ws: Entity<Workspace>,
    pub(super) vcx: VisualTestContext,
    pub(super) service: NotificationService,
    pub(super) recorder: Recorder,
    pub(super) wiring: NotificationWiring,
    _serve: Task<()>,
}

impl Fixture {
    pub(super) fn open(cx: &mut TestAppContext) -> Self {
        let (ws, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_keymap::init_with_text("", KeymapOptions::default(), cx);
        });
        let context = ClusterContext::new(
            prod(),
            ContextName::new("prod-eu"),
            SourceId("kubeconfig".into()),
        );
        let clock = Arc::new(FakeClockPort::default());
        let sessions = ClusterSessionManager::new(
            Arc::new(FakeClusterConnectorPort::new()),
            Arc::new(FakeClusterSourcePort::new().with_contexts([context.clone()])),
            clock.clone(),
        );
        sessions.open(&context, SessionOptions::default());
        let events = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
        let service = NotificationService::new(
            events,
            sessions.clone(),
            Arc::new(FakeStatePort::new()),
            clock,
        );
        let (sink, requests) = NotificationPanelSink::channel();
        let recorder = Recorder {
            sent: Rc::default(),
            service: service.clone(),
            sink,
        };
        let deps = NotificationDeps {
            service: service.clone(),
            sessions,
            dispatcher: Rc::new(recorder.clone()),
        };
        let wiring = vcx.update(|window, cx| install(&ws, deps, window, cx));
        let serve = vcx.update(|window, cx| serve(requests, ws.downgrade(), window, cx));
        vcx.run_until_parked();
        Self {
            ws,
            vcx,
            service,
            recorder,
            wiring,
            _serve: serve,
        }
    }

    /// Submits `input` and lets the window react.
    pub(super) fn submit(&mut self, input: NotificationInput) {
        self.service.submit(input);
        self.settle();
    }

    /// Runs what is ready, then lets the coalesced redraws land (no frames are drawn here, so
    /// the pacer's backstop delivers them).
    pub(super) fn settle(&mut self) {
        self.vcx.run_until_parked();
        self.vcx
            .executor()
            .advance_clock(oxikube_runtime::FRAME_STALL);
        self.vcx.run_until_parked();
    }

    /// Dispatches `command` as a button or the palette would.
    pub(super) fn dispatch(&mut self, command: Command) {
        let recorder = self.recorder.clone();
        self.vcx.update(|_, cx| recorder.dispatch(command, cx));
        self.settle();
    }

    pub(super) fn keys(&mut self, keys: &str) {
        self.vcx.simulate_keystrokes(keys);
        self.settle();
    }

    /// Whether the right dock (the panel's) is open.
    pub(super) fn panel_open(&mut self) -> bool {
        let ws = self.ws.clone();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .dock(DockPosition::Right, cx)
                .is_some_and(|dock| dock.is_open())
        })
    }

    /// Whether the panel has the focus.
    pub(super) fn panel_focused(&mut self) -> bool {
        let panel = self.wiring.panel.clone();
        self.vcx.update(|window, cx| {
            gpui::Focusable::focus_handle(panel.read(cx), cx).contains_focused(window, cx)
        })
    }

    /// The unread count the status-bar badge shows.
    pub(super) fn badge(&mut self) -> usize {
        let status = self.wiring.status.clone();
        self.vcx.update(|_, cx| status.read(cx).unread())
    }

    /// Opens the panel and draws the window.
    pub(super) fn show_panel(&mut self) {
        self.dispatch(Command::NotificationsTogglePanel);
        assert!(self.panel_open());
    }

    /// Clicks the element tagged `selector`.
    pub(super) fn click(&mut self, selector: &str) {
        // `debug_bounds` wants a static name; a test leaks a few bytes.
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        let bounds = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        self.vcx
            .simulate_click(bounds.center(), gpui::Modifiers::none());
        self.settle();
    }
}

/// The modifier of the global chords in the shipped keymap of this OS.
pub(super) fn modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    }
}
