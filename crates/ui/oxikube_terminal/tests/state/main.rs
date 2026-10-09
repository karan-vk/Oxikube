//! `#[gpui::test]` suite for the backend → grid → GPUI bridge (`TerminalState`, E09-S04), driven by
//! `FakeTerminalBackend` on the deterministic runtime (no OS threads).

mod coalescing;
mod frame_paced;
mod io;
mod lifecycle;
mod settings;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{AppContext as _, Entity, TestAppContext};
use oxikube_ports::TerminalSize;
use oxikube_runtime::FRAME_INTERVAL;
use oxikube_terminal::{TerminalEvent, TerminalState};
use oxikube_testkit::fakes::FakeTerminalBackend;

/// A terminal showing `backend`, plus a counter of observer notifications (repaints) and the
/// events it emitted.
struct Harness {
    terminal: Entity<TerminalState>,
    notifies: Rc<Cell<u32>>,
    events: Rc<RefCell<Vec<TerminalEvent>>>,
}

fn harness(cx: &mut TestAppContext, backend: &FakeTerminalBackend, size: (u16, u16)) -> Harness {
    cx.update(oxikube_runtime::init_deterministic);
    let boxed = Box::new(backend.clone());
    let terminal = cx.new(|cx| TerminalState::new(boxed, TerminalSize::new(size.0, size.1), cx));
    let notifies = Rc::new(Cell::new(0));
    let events = Rc::new(RefCell::new(Vec::new()));
    let (counter, sink) = (notifies.clone(), events.clone());
    cx.update(|cx| {
        cx.observe(&terminal, move |_, _| counter.set(counter.get() + 1))
            .detach();
        cx.subscribe(&terminal, move |_, event: &TerminalEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
    });
    cx.run_until_parked();
    Harness {
        terminal,
        notifies,
        events,
    }
}

/// Lets one frame pass: queued work runs, then the coalesced notify fires.
fn next_frame(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(FRAME_INTERVAL);
    cx.run_until_parked();
}

impl Harness {
    fn row(&self, cx: &mut TestAppContext, row: usize) -> String {
        self.terminal
            .read_with(cx, |terminal, _| terminal.snapshot().row_text(row))
    }
}
