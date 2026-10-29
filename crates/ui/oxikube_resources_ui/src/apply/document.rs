//! A document handed to the editor by "Open in editor".

use std::fmt;
use std::rc::Rc;
use std::sync::Arc;

use gpui::App;
use oxikube_domain::ids::ClusterId;

/// Text for a new editor tab. It may hold Secret values, so its `Debug` says its size only.
#[derive(Clone)]
pub struct EditorDocument {
    /// The cluster whose tab gets the editor.
    pub cluster: ClusterId,
    /// The tab title.
    pub title: String,
    /// The text.
    pub text: Arc<str>,
    /// The 1-based line the cursor goes to.
    pub line: usize,
}

impl fmt::Debug for EditorDocument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EditorDocument")
            .field("title", &self.title)
            .field("bytes", &self.text.len())
            .field("line", &self.line)
            .finish_non_exhaustive()
    }
}

/// Opens a document in the manifest editor (the app sends it to the window's editors).
pub type EditorOpener = Rc<dyn Fn(EditorDocument, &mut App)>;
