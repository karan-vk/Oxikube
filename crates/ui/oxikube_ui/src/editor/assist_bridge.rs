//! Plugs an [`EditorAssist`] into the library's hover and completion providers, and drives the
//! two popups from the keyboard (`editor::TriggerCompletion`, `editor::ShowHover`).
//!
//! This is the one place that names the language-server types the library takes. Typing asks for
//! completions through the library (`is_completion_trigger`, after a short pause so a burst of
//! keys costs one request); the pointer asks for hover through it (after its own delay). Every
//! request runs on the background executor over a shared-rope [`TextSnapshot`].

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use gpui::{App, Context, Task, Window};
use gpui_component::input::{CompletionProvider, HoverProvider, Rope, RopeExt as _};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Documentation, Hover, HoverContents, MarkupContent, MarkupKind, TextEdit,
};

use super::assist::{CompletionEntry, EditorAssist, EntryKind, HoverInfo};
use super::code_editor::{CodeEditor, TextSnapshot};

/// How long typing must pause before completions are computed. The task is dropped by the next
/// key, so a burst of typing computes nothing until it stops.
pub const COMPLETION_DEBOUNCE: Duration = Duration::from_millis(40);

/// The library's providers over an [`EditorAssist`].
struct Bridge(Arc<dyn EditorAssist>);

impl CompletionProvider for Bridge {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let assist = self.0.clone();
        let snapshot = TextSnapshot::of(text.clone(), 0);
        let executor = cx.background_executor().clone();
        let timer = executor.clone();
        executor.spawn(async move {
            timer.timer(COMPLETION_DEBOUNCE).await;
            let entries = assist.complete(&snapshot, offset);
            Ok(CompletionResponse::Array(items(&snapshot.rope, &entries)))
        })
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        self.0.triggers_completion(new_text)
    }
}

impl HoverProvider for Bridge {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        let assist = self.0.clone();
        let snapshot = TextSnapshot::of(text.clone(), 0);
        cx.background_executor().spawn(async move {
            Ok(assist
                .hover(&snapshot, offset)
                .map(|info| hover(&snapshot.rope, &info)))
        })
    }
}

fn range(rope: &Rope, bytes: &Range<usize>) -> lsp_types::Range {
    let len = rope.len();
    let start = bytes.start.min(len);
    lsp_types::Range::new(
        rope.offset_to_position(start),
        rope.offset_to_position(bytes.end.clamp(start, len)),
    )
}

fn markup(markdown: &str) -> MarkupContent {
    MarkupContent {
        kind: MarkupKind::Markdown,
        value: markdown.to_owned(),
    }
}

fn hover(rope: &Rope, info: &HoverInfo) -> Hover {
    Hover {
        contents: HoverContents::Markup(markup(&info.markdown)),
        range: Some(range(rope, &info.range)),
    }
}

fn items(rope: &Rope, entries: &[CompletionEntry]) -> Vec<CompletionItem> {
    entries
        .iter()
        .map(|entry| CompletionItem {
            label: entry.label.clone(),
            kind: Some(match entry.kind {
                EntryKind::Field => CompletionItemKind::FIELD,
                EntryKind::Value => CompletionItemKind::ENUM_MEMBER,
                EntryKind::Snippet => CompletionItemKind::SNIPPET,
            }),
            detail: entry.detail.clone(),
            documentation: entry
                .documentation
                .as_deref()
                .map(|doc| Documentation::MarkupContent(markup(doc))),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range: range(rope, &entry.replace),
                new_text: entry.insert_text.clone(),
            })),
            ..CompletionItem::default()
        })
        .collect()
}

impl CodeEditor {
    /// Hands `action` (Enter, Escape, Up or Down) to the open completion menu before the editor
    /// sees it, and stops it there.
    ///
    /// The library lets the menu consume these keys but leaves the action marked as propagating,
    /// so the key also reaches the editor (Enter would insert a newline after accepting an
    /// entry, Escape would reach the views around it). Doing it here, in the capture phase, makes
    /// the menu's key the whole of what the key does.
    pub(super) fn route_to_menu<A: gpui::Action + Clone>(
        &mut self,
        action: &A,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.read(cx).completion_menu_state().open {
            return;
        }
        let action = Box::new(action.clone());
        let handled = self.state.update(cx, |state, cx| {
            state.route_overlay_action(action, window, cx)
        });
        if handled {
            cx.stop_propagation();
        }
    }

    /// Gives the editor hover docs and completion (typed and [`show_completions`]) from `assist`.
    ///
    /// [`show_completions`]: CodeEditor::show_completions
    pub fn set_assist(&mut self, assist: Arc<dyn EditorAssist>, cx: &mut Context<Self>) {
        let bridge = Rc::new(Bridge(assist.clone()));
        self.assist = Some(assist);
        self.state.update(cx, |state, _| {
            let lsp = state.lsp_mut();
            lsp.completion_provider = Some(bridge.clone());
            lsp.hover_provider = Some(bridge);
        });
    }

    /// Opens the completion menu at the cursor now (`editor::TriggerCompletion`), without waiting
    /// for a key. The answer is dropped if the text or the cursor moved meanwhile. Does nothing
    /// in a read-only editor: accepting an entry would edit it.
    pub fn show_completions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(assist) = self.assist.clone().filter(|_| !self.read_only) else {
            return;
        };
        let (snapshot, offset) = self.cursor_snapshot(cx);
        let version = snapshot.version();
        self.assist_task = cx.spawn_in(window, async move |this, cx| {
            let background = snapshot.clone();
            let entries = cx
                .background_executor()
                .spawn(async move { assist.complete(&background, offset) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.version != version || entries.is_empty() {
                    return;
                }
                let start = entries
                    .iter()
                    .map(|e| e.replace.start)
                    .min()
                    .unwrap_or(offset)
                    .min(offset);
                let query = snapshot.text_between(start, offset);
                let items = items(&snapshot.rope, &entries);
                this.state.update(cx, |state, cx| {
                    if state.cursor() == offset {
                        state.present_completion_items(start, query, items, cx);
                    }
                });
            });
        });
    }

    /// The text now and the cursor in it, for a request that is answered later.
    fn cursor_snapshot(&self, cx: &gpui::App) -> (TextSnapshot, usize) {
        let state = self.state.read(cx);
        (
            TextSnapshot::of(state.text().clone(), self.version),
            state.cursor(),
        )
    }

    /// The labels of the completion menu now open, best first; empty when it is closed.
    pub fn open_completions(&self, cx: &gpui::App) -> Vec<String> {
        let menu = self.state.read(cx).completion_menu_state();
        if menu.open {
            menu.items.iter().map(|item| item.label.clone()).collect()
        } else {
            Vec::new()
        }
    }

    /// The Markdown of the hover popup now open and the bytes it is about, if one is.
    pub fn open_hover(&self, cx: &gpui::App) -> Option<(Range<usize>, String)> {
        let hover = self.state.read(cx).hover_popover()?;
        let markdown = match &hover.hover.contents {
            HoverContents::Markup(markup) => markup.value.clone(),
            _ => String::new(),
        };
        Some((hover.symbol_range.clone(), markdown))
    }

    /// Shows the docs of the token at the cursor (`editor::ShowHover`), as the pointer would.
    pub fn show_hover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(assist) = self.assist.clone() else {
            return;
        };
        let (snapshot, offset) = self.cursor_snapshot(cx);
        let version = snapshot.version();
        self.assist_task = cx.spawn_in(window, async move |this, cx| {
            let background = snapshot.clone();
            let found = cx
                .background_executor()
                .spawn(async move { assist.hover(&background, offset) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(info) = found.filter(|_| this.version == version) else {
                    return;
                };
                let docs = hover(&snapshot.rope, &info);
                this.state.update(cx, |state, cx| {
                    state.present_hover(info.range.clone(), docs, cx);
                });
            });
        });
    }
}
