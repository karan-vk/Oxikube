//! Drawing one cell: its text in the colour of its [`Tone`].
//!
//! A tone is a meaning (healthy, degraded, failed), never a colour: the colour comes from the
//! active theme's `oxikube` block (`OxikubeColors`), so a user theme recolours every status
//! cell, and without a theme (tests, a window before the theme loads) from the token colours
//! of `oxikube_ui`.

use gpui::{
    App, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    div,
};
use oxikube_app::columns::{Cell, Tone};
use oxikube_theme::ActiveTheme;
use oxikube_ui::ActiveTokens as _;

/// The text colour of each tone, read once per frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneColors {
    /// Plain cells.
    pub neutral: Hsla,
    /// `Running`, `Ready`, `Bound`.
    pub ok: Hsla,
    /// `Pending`, `Terminating`, `NotReady`.
    pub warn: Hsla,
    /// `Failed`, `CrashLoopBackOff`, `Error`.
    pub error: Hsla,
}

impl ToneColors {
    /// The colours of the active theme.
    pub fn current(cx: &App) -> Self {
        let colors = cx.colors();
        match cx.try_global::<ActiveTheme>() {
            Some(theme) => Self {
                neutral: colors.text,
                ok: theme.0.oxikube.status_running,
                warn: theme.0.oxikube.status_pending,
                error: theme.0.oxikube.status_failed,
            },
            None => Self {
                neutral: colors.text,
                ok: colors.success,
                warn: colors.warning,
                error: colors.error,
            },
        }
    }

    /// The colour of `tone`.
    pub fn of(&self, tone: Tone) -> Hsla {
        match tone {
            Tone::Neutral => self.neutral,
            Tone::Ok => self.ok,
            Tone::Warn => self.warn,
            Tone::Error => self.error,
        }
    }
}

/// The element of one cell (row `row`, shown column `col`): one line, cut with an ellipsis
/// when it does not fit. Tagged `cell-<row>-<col>` for test bounds.
pub fn cell_element(
    cell: &Cell<'_>,
    colors: &ToneColors,
    row: usize,
    col: usize,
) -> impl IntoElement + use<> {
    let text: SharedString = cell.display().to_owned().into();
    div()
        .debug_selector(move || format!("cell-{row}-{col}"))
        .w_full()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_color(colors.of(cell.tone()))
        .child(text)
}
