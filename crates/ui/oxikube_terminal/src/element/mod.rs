//! [`TerminalElement`]: the custom GPUI element that paints a [`TerminalState`] (E09-S05).
//!
//! Written from the xterm and GPUI documentation; Zed's `terminal_element.rs` was read for its
//! structure only and nothing is copied from it.
//!
//! | Phase | What |
//! |---|---|
//! | `request_layout` | fills its parent (`size_full`) |
//! | `prepaint` | cell metrics from the font; cols x rows from the bounds and, when they changed, one [`TerminalState::resize`]; a snapshot (never waiting on the grid lock); the palette from the theme; the row cache brought up to date; a hitbox |
//! | `paint` | background, cell backgrounds (merged spans), selection, text runs, cursor, decorations, the hovered link's underline, a scroll position thumb; then the mouse listeners |
//!
//! | Module | What |
//! |---|---|
//! | `metrics` | [`TerminalFont`], [`CellMetrics`]: cell size, pixels <-> cells |
//! | `palette` | [`TerminalPalette`]: theme terminal tokens -> cell colours (16 ANSI, 256, truecolour, dim, bold, inverse, hidden) |
//! | `layout` | a row as background spans, style runs and decoration spans (pure) |
//! | `cache` | the shaped rows of the previous frame, reused for rows whose content hash did not change, and the block cursor's glyph |
//! | `paint` | the paint passes |
//! | `links` | OSC 8, URL and path detection on the hovered line |
//! | `mouse` | hover, cmd/ctrl-click (dispatches `terminal::OpenLink`), selection drags, wheel scrolling |
//!
//! Keyboard input, IME and mouse reporting (E09-S06) are attached in the same paint:
//!
//! | Module | What |
//! |---|---|
//! | `attach` | the `Terminal` key context, the input handler (`EntityInputHandler` of the state), the key-down listener and the copy / paste actions ([`crate::input`]) |
//! | `preedit` | the IME composition painted inline at the cursor |
//! | `prepare` | per-frame upkeep of the state in prepaint: stale link hover, palette rebuild |
//! | `report` | mouse reporting to the process (SGR / UTF-8 / legacy; click, drag, motion, wheel; Shift bypasses) and alternate scroll |
//!
//! The view that hosts the element in a workspace tab is E09-S07.

mod attach;
mod cache;
mod hash;
mod layout;
pub mod links;
pub mod metrics;
mod mouse;
mod paint;
pub mod palette;
mod preedit;
mod prepare;
mod report;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::{
    App, Bounds, Element, ElementId, Entity, FocusHandle, GlobalElementId, Hitbox, HitboxBehavior,
    InspectorElementId, IntoElement, LayoutId, Pixels, Point, Style, Window, relative,
};
use oxikube_ports::TerminalSize;
use oxikube_theme::ActiveTheme;
use oxikube_workspace::CommandDispatcher;

use crate::grid::{SelectionSide, TerminalSnapshot};
use crate::input::{ImeAnchor, PasteConfirm};
use crate::state::TerminalState;

pub use cache::CacheStats;
pub use links::{LinkKind, TerminalLink};
pub use metrics::{CellMetrics, TerminalFont};
pub use palette::TerminalPalette;

use cache::RowCache;

/// Whether file paths in the output are links, and what relative ones are relative to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PathLinks {
    /// No path links (a pod's terminal: its paths are inside the container).
    #[default]
    Off,
    /// Paths are local files: absolute ones (and `~/`) open as they are, relative ones against
    /// `base` when it is known, otherwise they are not links.
    Local {
        /// The directory relative paths resolve against (the shell's working directory).
        base: Option<PathBuf>,
    },
}

/// What the element keeps between frames: the snapshot buffers, the shaped rows, the palette and
/// metrics it last used, and the hover and drag state. The host view creates one per terminal
/// and passes it to every [`TerminalElement`] it renders. Cheap to clone (a shared handle).
#[derive(Clone, Default)]
pub struct TerminalElementState(Rc<RefCell<Inner>>);

#[derive(Default)]
struct Inner {
    snapshot: TerminalSnapshot,
    cache: RowCache,
    /// The size the element last asked the terminal for.
    requested: Option<TerminalSize>,
    metrics: Option<(TerminalFont, CellMetrics)>,
    palette: Option<prepare::PaletteMemo>,
    hovered: Option<TerminalLink>,
    /// The content hash of each row the hovered link is on, when it was found.
    hover_rows: Vec<(usize, u64)>,
    /// The cell the pointer was last over (link detection runs when it changes).
    hover_cell: Option<(usize, usize)>,
    /// The cell a selection drag last reached; `None` when no drag is in progress.
    dragging: Option<(usize, usize, SelectionSide)>,
    /// Wheel movement not yet worth a whole line.
    scroll_remainder: Pixels,
    /// The button whose press was reported to the process (mouse reporting, E09-S06).
    reported: Option<crate::mappings::mouse::MouseButton>,
    /// The cell the last mouse report named (a report is sent once per cell).
    report_cell: Option<(usize, usize)>,
}

impl std::fmt::Debug for TerminalElementState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the snapshot: it holds what is on screen.
        f.debug_struct("TerminalElementState")
            .field("cache", &self.cache_stats())
            .finish_non_exhaustive()
    }
}

impl TerminalElementState {
    /// Fresh state: the first frame lays out and shapes every row.
    pub fn new() -> Self {
        Self::default()
    }

    /// The row cache's counters (tests and `--perf`).
    pub fn cache_stats(&self) -> CacheStats {
        self.0.borrow().cache.stats()
    }

    /// The link under the pointer while the platform modifier is held.
    pub fn hovered_link(&self) -> Option<TerminalLink> {
        self.0.borrow().hovered.clone()
    }
}

/// Paints a [`TerminalState`]. See the [module docs](self).
///
/// ```ignore
/// TerminalElement::new(&self.terminal, &self.element_state, &self.focus)
///     .dispatcher(self.dispatcher.clone())
///     .path_links(PathLinks::Local { base: Some(cwd) })
/// ```
pub struct TerminalElement {
    terminal: Entity<TerminalState>,
    state: TerminalElementState,
    focus: FocusHandle,
    font: Option<TerminalFont>,
    dispatcher: Option<Rc<dyn CommandDispatcher>>,
    paths: PathLinks,
    confirm: Option<Rc<dyn PasteConfirm>>,
}

impl TerminalElement {
    /// An element painting `terminal`, keeping its frame-to-frame state in `state`, drawing the
    /// focused cursor while `focus` is focused (a hollow block otherwise).
    pub fn new(
        terminal: &Entity<TerminalState>,
        state: &TerminalElementState,
        focus: &FocusHandle,
    ) -> Self {
        Self {
            terminal: terminal.clone(),
            state: state.clone(),
            focus: focus.clone(),
            font: None,
            dispatcher: None,
            paths: PathLinks::Off,
            confirm: None,
        }
    }

    /// Where a multi-line paste asks for confirmation (`terminal.confirm_multiline_paste`).
    /// Without one the paste goes ahead unasked, so a host that wants the safety net sets it
    /// (for example [`WorkspacePasteConfirm`](crate::input::WorkspacePasteConfirm)).
    #[must_use]
    pub fn paste_confirm(mut self, confirm: Rc<dyn PasteConfirm>) -> Self {
        self.confirm = Some(confirm);
        self
    }

    /// Draws with `font` instead of the theme's monospace font.
    #[must_use]
    pub fn font(mut self, font: TerminalFont) -> Self {
        self.font = Some(font);
        self
    }

    /// Where cmd/ctrl-click sends `terminal::OpenLink`. Without one, links are shown on hover but
    /// clicking them does nothing.
    #[must_use]
    pub fn dispatcher(mut self, dispatcher: Rc<dyn CommandDispatcher>) -> Self {
        self.dispatcher = Some(dispatcher);
        self
    }

    /// Whether paths in the output are links (off by default).
    #[must_use]
    pub fn path_links(mut self, paths: PathLinks) -> Self {
        self.paths = paths;
        self
    }
}

/// What prepaint hands to paint.
pub struct TerminalFrame {
    hitbox: Hitbox,
    origin: Point<Pixels>,
    metrics: CellMetrics,
    focused: bool,
    /// The IME composition to paint at the cursor, if one is in progress.
    preedit: Option<String>,
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = TerminalFrame;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> TerminalFrame {
        let font = self
            .font
            .clone()
            .unwrap_or_else(|| TerminalFont::from_theme(cx));
        let mut inner = self.state.0.borrow_mut();
        let metrics = match &inner.metrics {
            Some((measured, metrics)) if *measured == font => *metrics,
            _ => {
                let metrics = CellMetrics::measure(&font, window.text_system());
                inner.metrics = Some((font.clone(), metrics));
                metrics
            }
        };

        // Resize first, so this frame's snapshot already has the new size (resize <= 1 frame).
        let size = metrics.grid_size(bounds.size);
        if inner.requested != Some(size) {
            inner.requested = Some(size);
            self.terminal
                .update(cx, |terminal, cx| terminal.resize(size, cx));
        }
        if self
            .terminal
            .read(cx)
            .try_snapshot_into(&mut inner.snapshot)
        {
            prepare::forget_stale_hover(&mut inner);
        } else {
            // The grid is busy with a parse or search slice: paint the last frame, try again.
            window.request_animation_frame();
        }

        // Tell the state where the cursor is (the IME candidate window goes there), and fetch the
        // composition to paint. Neither notifies: nothing here changes what is painted.
        let anchor = ImeAnchor {
            cell: gpui::size(metrics.cell_width, metrics.line_height),
            row: inner.snapshot.cursor.row,
            column: inner.snapshot.cursor.column,
        };
        let preedit = self.terminal.update(cx, |terminal, _| {
            if terminal.ime_anchor() != Some(anchor) {
                terminal.set_ime_anchor(anchor);
            }
            terminal
                .composition()
                .map(|composition| composition.text.clone())
        });

        prepare::refresh_palette(&mut inner, ActiveTheme::get(cx));
        let Inner {
            snapshot,
            cache,
            palette,
            ..
        } = &mut *inner;
        if let Some(memo) = palette {
            cache.update(
                snapshot,
                &font,
                metrics,
                &memo.palette,
                window.text_system(),
            );
        }
        drop(inner);

        // The element is the focus target: modifier changes (link hover) and, from E09-S06, keys
        // reach it while its handle is focused.
        window.set_focus_handle(&self.focus, cx);
        TerminalFrame {
            hitbox: window.insert_hitbox(bounds, HitboxBehavior::Normal),
            origin: bounds.origin,
            metrics,
            focused: self.focus.is_focused(window),
            preedit,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        frame: &mut TerminalFrame,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            paint::paint(&self.state, frame, bounds, window, cx);
        });
        attach::register(self, bounds, window, cx);
        mouse::register(self, frame, window);
    }
}
