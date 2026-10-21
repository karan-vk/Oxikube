//! `#[gpui::test]`s of the capability gate, the banner and the Helm page in a workspace window,
//! over scripted [`HelmCapabilities`] (no `helm` binary needed).

mod banner;
mod gate;
mod page;

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{AppContext as _, Entity, Modifiers, Point, TestAppContext, VisualTestContext};
use oxikube_domain::command::Command;
use oxikube_domain::helm::{HelmCapabilities, HelmVersion};
use oxikube_workspace::test_support::open_workspace;
use oxikube_workspace::{CommandDispatcher, OpenOptions};

use crate::{HelmGateState, HelmPage};

/// Records every command the banner's buttons send.
#[derive(Clone, Default)]
pub(crate) struct Recorder(Rc<RefCell<Vec<Command>>>);

impl Recorder {
    pub fn sent(&self) -> Vec<Command> {
        self.0.borrow().clone()
    }
}

impl CommandDispatcher for Recorder {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.0.borrow_mut().push(command);
    }
}

pub(crate) fn full() -> HelmCapabilities {
    HelmCapabilities::full(HelmVersion::new(3, 14, 2))
}

/// The Helm page open in a workspace window.
pub(crate) struct Harness {
    pub vcx: VisualTestContext,
    pub state: Entity<HelmGateState>,
    pub sent: Recorder,
}

impl Harness {
    /// Opens the page with a native reader mounted and `cli` as the binary's answer.
    pub fn open(cx: &mut TestAppContext, cli: HelmCapabilities) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        let sent = Recorder::default();
        let mut parts = None;
        vcx.update(|window, cx| {
            let state = HelmGateState::install(cx);
            state.update(cx, |state, cx| {
                state.set_native_read(true, cx);
                state.set_cli_capabilities(cli, cx);
            });
            let page = cx.new(|cx| HelmPage::new(state.clone(), Rc::new(sent.clone()), cx));
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(page.clone()), OpenOptions::default(), window, cx);
            });
            parts = Some(state);
        });
        vcx.run_until_parked();
        let state = parts.expect("the page was built");
        Self { vcx, state, sent }
    }

    /// Lets the page follow a new detection result.
    pub fn detected(&mut self, cli: HelmCapabilities) {
        let state = self.state.clone();
        self.vcx
            .update(|_, cx| state.update(cx, |s, cx| s.set_cli_capabilities(cli, cx)));
        self.vcx.run_until_parked();
    }

    pub fn is_laid_out(&mut self, selector: &'static str) -> bool {
        self.vcx.debug_bounds(selector).is_some()
    }

    pub fn click(&mut self, selector: &'static str) {
        let at: Point<_> = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center();
        self.vcx.simulate_click(at, Modifiers::none());
        self.vcx.run_until_parked();
    }
}
