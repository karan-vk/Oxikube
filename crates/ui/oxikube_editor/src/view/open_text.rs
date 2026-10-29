//! "Open in editor" from the apply panel (E20-S05): a document's text in a new editor tab with
//! the cursor on the line it starts at.
//!
//! The tab holds the text the panel gives it (a file read again, or the object rendered back to
//! YAML for kustomize output) with the cluster's schemas, so the usual checks and the apply of a
//! buffer (`resource::Apply`) work on it. The text may hold Secret values: it is never logged
//! (the [`Debug`] of [`OpenText`] says its size) and never saved.

use std::fmt;
use std::sync::Arc;

use gpui::{AppContext as _, Context, SharedString, Window};
use oxikube_domain::ids::ClusterId;
use oxikube_ui::editor::EditorApi as _;

use super::controller::EditorViews;
use super::manifest_editor::{ManifestEditor, ManifestEditorParts};

/// Text to open in a new editor tab.
#[derive(Clone, PartialEq, Eq)]
pub struct OpenText {
    /// The cluster whose tab gets the editor.
    pub cluster: ClusterId,
    /// The tab title (the file name).
    pub title: String,
    /// The text.
    pub text: Arc<str>,
    /// The 1-based line the cursor is put on.
    pub line: usize,
}

impl fmt::Debug for OpenText {
    /// The text may hold Secret values: only its size is shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenText")
            .field("cluster", &self.cluster)
            .field("title", &self.title)
            .field("bytes", &self.text.len())
            .field("line", &self.line)
            .finish()
    }
}

/// The byte offset where 1-based `line` of `text` starts (the end of `text` past its last line).
pub fn line_start(text: &str, line: usize) -> usize {
    let mut offset = 0;
    for _ in 1..line.max(1) {
        match text[offset..].find('\n') {
            Some(newline) => offset += newline + 1,
            None => return text.len(),
        }
    }
    offset
}

impl EditorViews {
    /// Opens an editor on `open.text` in its cluster's tab with the cursor on `open.line`. A
    /// cluster with no tab here does nothing.
    pub(super) fn open_text(
        &mut self,
        open: OpenText,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.deps.host.clone();
        let Some(workspace) = host.workspace(&open.cluster, cx) else {
            tracing::debug!("open in editor: the cluster has no tab in this window");
            return;
        };
        host.show(&open.cluster, window, cx);
        let start = line_start(&open.text, open.line);
        let parts = ManifestEditorParts {
            title: SharedString::from(open.title),
            text: open.text.to_string(),
            schemas: Some(host.schemas(&open.cluster)),
            live: host.live(&open.cluster),
            dispatcher: self.deps.dispatcher.clone(),
            resource: None,
        };
        let editor = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        self.mount(&editor, Some(&open.cluster), &workspace, cx);
        workspace.update(cx, |ws, cx| ws.open_item(editor.clone(), window, cx));
        let code = editor.read(cx).editor().clone();
        code.update(cx, |code, cx| {
            code.api(window, cx).select(start..start);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::line_start;

    #[test]
    fn a_line_starts_after_the_newlines_before_it() {
        let text = "a\nbb\nccc\n";
        assert_eq!(line_start(text, 1), 0);
        assert_eq!(line_start(text, 2), 2);
        assert_eq!(line_start(text, 3), 5);
        assert_eq!(line_start(text, 0), 0, "line 0 is read as line 1");
        assert_eq!(line_start(text, 99), text.len(), "past the end is the end");
    }
}
