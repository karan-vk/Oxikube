//! [`EditorApi`]: what a feature view may do with a code editor, in our own types.
//!
//! The manifest editor (`oxikube_editor`), and later the Helm values editor, talk to this trait
//! only, so no feature crate names a gpui-component type. Every position is a **UTF-8 byte
//! offset** into the buffer (what the spanned YAML model and the validator produce); the one
//! place that converts offsets to the library's line/column positions is `super::positions`.
//!
//! The trait takes no GPUI context: the live implementation ([`LiveEditor`](super::LiveEditor))
//! borrows the window and the app for the length of one call sequence, and a test can implement
//! it with a plain struct.

use std::ops::Range;
use std::sync::Arc;

use gpui::{Bounds, Pixels};

/// How serious an [`EditorDiagnostic`] is: the squiggle's colour and the gutter marker's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticLevel {
    /// The text is wrong (red).
    Error,
    /// The text is probably wrong (amber).
    Warning,
    /// Something to know (blue).
    Info,
    /// A suggestion (faint).
    Hint,
}

/// One finding to draw: a squiggle under `range`, a marker in the gutter of its first line and
/// the message at the end of that line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorDiagnostic {
    /// Bytes of the buffer to underline. Clamped to the buffer and to character boundaries when
    /// drawn; an empty range underlines the character after it.
    pub range: Range<usize>,
    /// Error, warning, info or hint.
    pub level: DiagnosticLevel,
    /// What is wrong, in one line.
    pub message: String,
    /// A stable code for the rule (`unknown-field`), shown in the hover.
    pub code: Option<String>,
}

impl EditorDiagnostic {
    /// A finding with no code.
    pub fn new(range: Range<usize>, level: DiagnosticLevel, message: impl Into<String>) -> Self {
        Self {
            range,
            level,
            message: message.into(),
            code: None,
        }
    }

    /// Sets the code.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

/// How a decoration draws its range. Decorations never change the text or its layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecorationStyle {
    /// The text is drawn faded (a field the server owns, such as `status`).
    Dimmed,
    /// A filled background behind the range.
    Highlight,
    /// A one-pixel frame around the range.
    Frame,
}

/// A decoration: a style over a byte range. Ranges follow later edits of the buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decoration {
    /// Bytes of the buffer.
    pub range: Range<usize>,
    /// How the range is drawn.
    pub style: DecorationStyle,
}

/// What a code editor offers a feature view. Offsets are UTF-8 byte offsets.
pub trait EditorApi {
    /// The whole buffer as one string. Allocates: call it when the text is needed (a debounced
    /// validation, an apply), not per frame.
    fn text(&self) -> Arc<str>;

    /// A number that grows with every change of the buffer (typing, undo, [`set_text`]). Results
    /// computed for an older version are stale.
    ///
    /// [`set_text`]: EditorApi::set_text
    fn version(&self) -> u64;

    /// Replaces the whole buffer. Clears the undo history and the diagnostics; the scroll
    /// position stays.
    fn set_text(&mut self, text: &str);

    /// The selections, the primary one first; an empty range is a cursor.
    fn selections(&self) -> Vec<Range<usize>>;

    /// Selects `range` (a cursor when empty) and scrolls it into view.
    fn select(&mut self, range: Range<usize>);

    /// Whether typing is refused.
    fn is_read_only(&self) -> bool;

    /// Refuses typing, or allows it again. A view state: it says nothing about the cluster.
    fn set_read_only(&mut self, read_only: bool);

    /// Whether long lines wrap at the editor's width.
    fn soft_wrap(&self) -> bool;

    /// Wraps long lines at the editor's width, or lets them scroll sideways.
    fn set_soft_wrap(&mut self, wrap: bool);

    /// Replaces the diagnostics: squiggles, gutter markers and end-of-line messages. They are
    /// dropped by the next edit, so set them for the [`version`](EditorApi::version) they were
    /// computed for only.
    fn set_diagnostics(&mut self, diagnostics: Vec<EditorDiagnostic>);

    /// The diagnostics shown now, in buffer order.
    fn diagnostics(&self) -> Vec<EditorDiagnostic>;

    /// Adds a decoration.
    fn add_decoration(&mut self, decoration: Decoration);

    /// The decorations now, where the edits since they were added moved them; a decoration whose
    /// text was deleted is gone.
    fn decorations(&self) -> Vec<Decoration>;

    /// Removes every decoration.
    fn clear_decorations(&mut self);

    /// Where `range` was drawn in the last frame, in window coordinates; `None` before the first
    /// frame or when the range is outside the visible rows. For overlays.
    fn range_to_bounds(&self, range: Range<usize>) -> Option<Bounds<Pixels>>;
}
