//! The element's mouse handling: link hover and cmd/ctrl-click, selection drags and wheel
//! scrolling through the history.
//!
//! * **Links**: with the platform modifier held (cmd on macOS, ctrl elsewhere) the link under the
//!   pointer is found ([`links::link_at`], only when the hovered cell or the modifier changes),
//!   underlined, and shown with a pointing-hand cursor. A click with the modifier dispatches
//!   `Command::TerminalOpenLink` through the element's [`CommandDispatcher`]; nothing is opened
//!   directly.
//! * **Selection**: a press starts one (double click: words, triple: lines, alt: a block,
//!   shift: extends the current one), a drag extends it, the release ends the drag.
//! * **Wheel**: scrolls the view through the history, whole lines at a time. On the alternate
//!   screen the wheel belongs to the application (mouse reporting is E09-S06).

use std::rc::Rc;

use gpui::{
    App, CursorStyle, DispatchPhase, Entity, FocusHandle, Hitbox, Modifiers, ModifiersChangedEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent,
    Window,
};
use oxikube_domain::command::Command;
use oxikube_workspace::CommandDispatcher;

use super::metrics::CellMetrics;
use super::{PathLinks, TerminalElement, TerminalElementState, TerminalFrame};
use super::{hash, links};
use crate::grid::{GridPoint, SelectionKind, SelectionSide, TerminalModes, TerminalScroll};
use crate::state::TerminalState;

/// What every listener of one frame needs.
struct Pointer {
    terminal: Entity<TerminalState>,
    state: TerminalElementState,
    focus: FocusHandle,
    dispatcher: Option<Rc<dyn CommandDispatcher>>,
    paths: PathLinks,
    hitbox: Hitbox,
    origin: Point<Pixels>,
    metrics: CellMetrics,
}

/// Sets the cursor style and registers this frame's mouse listeners.
pub(super) fn register(element: &TerminalElement, frame: &TerminalFrame, window: &mut Window) {
    let over_link = element.state.0.borrow().hovered.is_some();
    let style = if over_link {
        CursorStyle::PointingHand
    } else {
        CursorStyle::IBeam
    };
    window.set_cursor_style(style, &frame.hitbox);

    let pointer = Rc::new(Pointer {
        terminal: element.terminal.clone(),
        state: element.state.clone(),
        focus: element.focus.clone(),
        dispatcher: element.dispatcher.clone(),
        paths: element.paths.clone(),
        hitbox: frame.hitbox.clone(),
        origin: frame.origin,
        metrics: frame.metrics,
    });
    let p = pointer.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
            p.moved(event, window, cx);
        }
    });
    let p = pointer.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            p.pressed(event, window, cx);
        }
    });
    let p = pointer.clone();
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, _| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            p.state.0.borrow_mut().dragging = None;
        }
    });
    let p = pointer.clone();
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
            p.scrolled(event, window, cx);
        }
    });
    window.on_modifiers_changed(move |event: &ModifiersChangedEvent, window, cx| {
        let position = window.mouse_position();
        pointer.hover(position, event.modifiers, window, cx);
    });
}

impl Pointer {
    /// The viewport cell under `position`, and the half of it the pointer is on.
    fn cell(&self, position: Point<Pixels>) -> (usize, usize, SelectionSide) {
        let inner = self.state.0.borrow();
        let snapshot = &inner.snapshot;
        self.metrics
            .cell_at(self.origin, position, snapshot.columns, snapshot.rows)
    }

    fn grid_point(&self, row: usize, column: usize) -> GridPoint {
        let offset = self.state.0.borrow().snapshot.display_offset;
        GridPoint::from_viewport(row, column, offset)
    }

    fn moved(&self, event: &MouseMoveEvent, window: &mut Window, cx: &mut App) {
        let dragging = self.state.0.borrow().dragging;
        if let Some(last) = dragging
            && event.pressed_button == Some(MouseButton::Left)
        {
            let cell = self.cell(event.position);
            // Only a move to another cell (or half of one) changes the selection and repaints.
            if cell != last {
                self.state.0.borrow_mut().dragging = Some(cell);
                let (row, column, side) = cell;
                let point = self.grid_point(row, column);
                self.terminal.update(cx, |terminal, cx| {
                    terminal.update_selection(point, side, cx)
                });
            }
        }
        self.hover(event.position, event.modifiers, window, cx);
    }

    /// Finds (or drops) the hovered link; repaints when it changed.
    fn hover(&self, position: Point<Pixels>, modifiers: Modifiers, window: &mut Window, _: &App) {
        let wanted = modifiers.secondary() && self.hitbox.is_hovered(window);
        let mut inner = self.state.0.borrow_mut();
        if !wanted {
            inner.hover_cell = None;
            if inner.hovered.take().is_some() {
                window.refresh();
            }
            return;
        }
        let snapshot = &inner.snapshot;
        let (row, column, _) =
            self.metrics
                .cell_at(self.origin, position, snapshot.columns, snapshot.rows);
        if inner.hover_cell == Some((row, column)) {
            return;
        }
        let paths = matches!(self.paths, PathLinks::Local { .. });
        let found = links::link_at(snapshot, row, column, paths);
        let rows = found
            .iter()
            .flat_map(|link| &link.cells)
            .map(|&(row, _, _)| (row, hash::row_hash(snapshot, row)));
        let rows: Vec<(usize, u64)> = rows.collect();
        inner.hover_cell = Some((row, column));
        inner.hover_rows = rows;
        if inner.hovered != found {
            inner.hovered = found;
            window.refresh();
        }
    }

    fn pressed(&self, event: &MouseDownEvent, window: &mut Window, cx: &mut App) {
        if !self.hitbox.is_hovered(window) {
            return;
        }
        self.focus.focus(window, cx);
        let (row, column, side) = self.cell(event.position);
        if event.modifiers.secondary() {
            self.open_link(row, column, cx);
            cx.stop_propagation();
            return;
        }
        let point = self.grid_point(row, column);
        let extend = event.modifiers.shift;
        let kind = match event.click_count {
            2 => SelectionKind::Word,
            n if n >= 3 => SelectionKind::Line,
            _ if event.modifiers.alt => SelectionKind::Block,
            _ => SelectionKind::Cell,
        };
        self.state.0.borrow_mut().dragging = Some((row, column, side));
        self.terminal.update(cx, |terminal, cx| {
            if extend {
                terminal.update_selection(point, side, cx);
            } else {
                terminal.start_selection(kind, point, side, cx);
            }
        });
    }

    /// Dispatches `terminal::OpenLink` for the link at viewport `row`, `column`, if any.
    fn open_link(&self, row: usize, column: usize, cx: &mut App) {
        let Some(dispatcher) = &self.dispatcher else {
            return;
        };
        let (paths, base) = match &self.paths {
            PathLinks::Off => (false, None),
            PathLinks::Local { base } => (true, base.as_deref()),
        };
        let link = {
            let inner = self.state.0.borrow();
            links::link_at(&inner.snapshot, row, column, paths)
        };
        if let Some(target) = link.and_then(|link| link.resolve(base)) {
            dispatcher.dispatch(Command::TerminalOpenLink { target }, cx);
        }
    }

    fn scrolled(&self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut App) {
        if !self.hitbox.is_hovered(window) {
            return;
        }
        let lines = {
            let mut inner = self.state.0.borrow_mut();
            if inner.snapshot.modes.contains(TerminalModes::ALT_SCREEN) {
                return;
            }
            let line_height = self.metrics.line_height;
            let delta = event.delta.pixel_delta(line_height).y + inner.scroll_remainder;
            let lines = (delta / line_height).trunc();
            inner.scroll_remainder = delta - line_height * lines;
            lines as i32
        };
        if lines != 0 {
            self.terminal.update(cx, |terminal, cx| {
                terminal.scroll(TerminalScroll::Lines(lines), cx)
            });
        }
    }
}
