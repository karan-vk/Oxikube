//! The user's own config files in the editor (E21-S03): `keymap.json` opened in a
//! [`ManifestEditor`] is checked by the keymap validator instead of the Kubernetes schemas.
//!
//! - [`ConfigFileKind`] is the association from a path to what checks it. Only the keymap is
//!   associated so far; `settings.json` joins in the settings stories.
//! - [`validate_config`] is the pass: the pure keymap validator
//!   ([`oxikube_keymap::validate`], the same function the keymap's own load path calls) on a
//!   snapshot of the names it needs, turned into editor diagnostics. It runs on the background
//!   executor after the same [`VALIDATION_DEBOUNCE`](super::VALIDATION_DEBOUNCE) as a manifest, and a
//!   result for an older buffer version is dropped.
//! - Saving (`resource::Apply`, cmd-s / ctrl-s) writes the file; the keymap's file watcher
//!   reloads it, so the notification and the live rebinding follow with no second path. The save
//!   goes through [`oxikube_keymap::edit::save_user_keymap_text`]: under the lock the keymap
//!   editor's writes take, and only while the file still holds the text the buffer started from.
//!   When another writer (a binding recorded in the keymap editor) changed it, nothing is written
//!   and a warning says so; saving again overwrites knowingly.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, AppContext as _, Context, SharedString, Window};
use oxikube_keymap::{ActionRegistry, KeymapDiagnostic, KnownNames, Severity};
use oxikube_ui::editor::{DiagnosticLevel, EditorDiagnostic};
use oxikube_workspace::Toast;

use super::controller::EditorViews;
use super::manifest_editor::{ManifestEditor, ManifestEditorParts};
use super::notice::{Notice, NoticeLevel};
use super::progressive::Coverage;
use super::validation::Validation;
use crate::cancel::{CancelOnDrop, CancelToken};
use crate::json::{Language, LanguageMode};

/// What checks a config file opened in the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigFileKind {
    /// The user's `keymap.json`: checked by the keymap validator.
    Keymap,
}

impl ConfigFileKind {
    /// The kind of the file at `path`, when it is one of the user's config files: the keymap's
    /// own `keymap.json` (`oxikube_keymap::user_keymap_file`).
    pub fn of_path(path: &Path, cx: &App) -> Option<Self> {
        let keymap = oxikube_keymap::user_keymap_file(cx)?;
        (keymap == path).then_some(Self::Keymap)
    }

    /// The tab title: the file's name.
    pub fn title(self) -> &'static str {
        match self {
            Self::Keymap => "keymap.json",
        }
    }
}

/// A config file an editor is editing: what it is and where it is saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    /// What checks the buffer.
    pub kind: ConfigFileKind,
    /// Where Save writes.
    pub path: PathBuf,
}

/// The config file an editor edits and the text on disk its buffer started from (what it was
/// opened with, or last saved or warned about), so a save can tell another writer changed it.
#[derive(Clone, Debug)]
pub(super) struct ConfigFileLink {
    pub(super) file: ConfigFile,
    pub(super) on_disk: String,
}

/// Text to open in an editor as the user's config file `file` (the keymap's `keymap::OpenUser`
/// sends it). Not a command: the command is `keymap::OpenUser`; this is how its result reaches
/// the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenConfigFile {
    /// The file to read and edit.
    pub file: ConfigFile,
}

/// What a keymap pass needs, copied so it can run on the background executor.
#[derive(Clone, Debug)]
pub struct KeymapNames {
    actions: ActionRegistry,
    contexts: KnownNames,
}

impl KeymapNames {
    /// The actions registered in the running app and the context names known now.
    pub fn capture(cx: &App) -> Self {
        Self {
            actions: ActionRegistry::from_app(cx),
            contexts: KnownNames::current(),
        }
    }

    /// Exactly these names, for tests.
    pub fn new(actions: ActionRegistry, contexts: KnownNames) -> Self {
        Self { actions, contexts }
    }
}

/// The keymap validator's findings in `text` as editor diagnostics: a squiggle under the token
/// each is about (its line when it has no span), the message with its "did you mean", and the
/// validator's code.
pub fn keymap_diagnostics(text: &str, names: &KeymapNames) -> Vec<EditorDiagnostic> {
    oxikube_keymap::validate(text, &names.actions, &names.contexts)
        .iter()
        .map(|found| editor_diagnostic(text, found))
        .collect()
}

fn editor_diagnostic(text: &str, found: &KeymapDiagnostic) -> EditorDiagnostic {
    let level = match found.severity() {
        Severity::Error => DiagnosticLevel::Error,
        Severity::Warning => DiagnosticLevel::Warning,
    };
    let range = found.span.clone().unwrap_or_else(|| {
        let start = oxikube_keymap::validate::offset_of(text, found);
        start..start
    });
    EditorDiagnostic::new(range, level, found.message()).with_code(found.code())
}

/// One pass over `text` (the buffer at `version`) of a config file of `kind`, as a [`Validation`].
pub fn validate_config(
    kind: ConfigFileKind,
    version: u64,
    text: &str,
    names: &KeymapNames,
    mode: LanguageMode,
) -> Validation {
    let diagnostics = match kind {
        ConfigFileKind::Keymap => keymap_diagnostics(text, names),
    };
    Validation {
        version,
        mode,
        language: Language::Json,
        diagnostics,
        missing: Vec::new(),
        coverage: Coverage { done: 1, total: 1 },
    }
}

impl EditorViews {
    /// Opens `open.file` in an editor in the workspace on screen, or shows the one that has it.
    /// The file is read off the UI thread.
    pub(super) fn open_config_file(
        &mut self,
        open: OpenConfigFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.shown_workspace(cx) else {
            return;
        };
        let held = workspace
            .read(cx)
            .items_of_type::<ManifestEditor>()
            .into_iter()
            .find(|editor| editor.read(cx).config_file() == Some(&open.file));
        if let Some(editor) = held {
            workspace.update(cx, |ws, cx| {
                ws.activate_item(editor.entity_id(), true, window, cx);
            });
            return;
        }
        let path = open.file.path.clone();
        self.file_read = Some(cx.spawn_in(window, async move |this, cx| {
            let text = cx
                .background_spawn(async move { std::fs::read_to_string(&path) })
                .await;
            let _ = this.update_in(cx, |views, window, cx| {
                views.show_config_file(open.file, text, window, cx);
            });
        }));
    }

    fn show_config_file(
        &mut self,
        file: ConfigFile,
        text: std::io::Result<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.shown_workspace(cx) else {
            return;
        };
        let text = match text {
            Ok(text) => text,
            Err(error) => {
                let toast =
                    Toast::warning(format!("Could not open {}: {error}", file.kind.title()));
                workspace.update(cx, |workspace, cx| workspace.show_toast(toast, cx));
                return;
            }
        };
        let kind = file.kind;
        let on_disk = text.clone();
        let parts = ManifestEditorParts {
            title: SharedString::from(kind.title()),
            text,
            schemas: None,
            live: None,
            dispatcher: self.deps.dispatcher.clone(),
            resource: None,
        };
        let editor = cx.new(|cx| ManifestEditor::new(parts, window, cx));
        self.mount(&editor, None, &workspace, cx);
        editor.update(cx, |editor, cx| {
            editor.set_config_file(file, on_disk, window, cx);
            editor.set_notice(opened_notice(kind), cx);
        });
        workspace.update(cx, |ws, cx| ws.open_item(editor, window, cx));
    }
}

impl ManifestEditor {
    /// Makes this editor edit the config file `file`, whose text on disk is `on_disk` (what the
    /// buffer was opened with): its buffer is read as JSON, checked by the file's validator
    /// instead of the Kubernetes schemas, and saved to its path (`resource::Apply`) unless another
    /// writer changed the file since.
    pub fn set_config_file(
        &mut self,
        file: ConfigFile,
        on_disk: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.config_file = Some(ConfigFileLink { file, on_disk });
        self.use_language(Language::Json, window, cx);
    }

    /// The config file this editor edits, if it is one.
    pub fn config_file(&self) -> Option<&ConfigFile> {
        self.config_file.as_ref().map(|link| &link.file)
    }

    /// Validates the buffer as a config file after `delay`: the same debounce, snapshot and
    /// stale-result rules as a manifest pass (see `pipeline`), with the file's validator.
    pub(super) fn schedule_config_validation(
        &mut self,
        kind: ConfigFileKind,
        delay: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cancel = CancelToken::new();
        self.pass_cancel = CancelOnDrop::new(cancel.clone());
        self.validation = Some(cx.spawn_in(window, async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let Ok((snapshot, names, mode)) = this.update(cx, |view, cx| {
                view.pass_running = true;
                view.model.stats_mut().started += 1;
                (
                    view.editor.read(cx).snapshot(cx),
                    KeymapNames::capture(cx),
                    view.model.mode(),
                )
            }) else {
                return;
            };
            let version = snapshot.version();
            let token = cancel.clone();
            let validation = cx
                .background_spawn(async move {
                    let text = snapshot.to_text();
                    (!token.is_cancelled())
                        .then(|| validate_config(kind, version, &text, &names, mode))
                })
                .await;
            let Some(validation) = validation else {
                return;
            };
            let _ = this.update_in(cx, |view, window, cx| {
                let model = &mut view.model;
                let accepted = view.editor.update(cx, |editor, cx| {
                    model.accept(&mut editor.api(window, cx), validation)
                });
                view.pass_running = false;
                if accepted == super::model::Accepted::Shown {
                    view.model.stats_mut().completed += 1;
                }
                cx.notify();
            });
        }));
    }
}

/// Shared so the controller and the tests build the same notice when a file opens.
pub(super) fn opened_notice(kind: ConfigFileKind) -> Notice {
    let text = match kind {
        ConfigFileKind::Keymap => {
            "Editing keymap.json: problems are underlined; save with cmd-s / ctrl-s"
        }
    };
    Notice::new(NoticeLevel::Info, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> KeymapNames {
        KeymapNames::new(
            ActionRegistry::from_names(["table::SelectNext"]),
            KnownNames::current(),
        )
    }

    #[test]
    fn findings_become_underlines_with_levels_codes_and_suggestions() {
        let text =
            "[{\"context\": \"ResourceTabel\", \"bindings\": {\"j\": \"table::SelctNext\"}}]";
        let found = keymap_diagnostics(text, &names());
        assert_eq!(found.len(), 2);
        let context = &found[0];
        assert_eq!(context.level, DiagnosticLevel::Warning);
        assert_eq!(&text[context.range.clone()], "\"ResourceTabel\"");
        assert_eq!(context.code.as_deref(), Some("keymap/unknown-context"));
        assert!(context.message.contains("did you mean `ResourceTable`?"));
        let action = &found[1];
        assert_eq!(action.level, DiagnosticLevel::Error);
        assert_eq!(&text[action.range.clone()], "\"j\"");
        assert!(action.message.contains("table::SelectNext"));
    }

    #[test]
    fn the_pass_carries_the_version_and_a_complete_coverage() {
        let result = validate_config(
            ConfigFileKind::Keymap,
            7,
            "[]",
            &names(),
            LanguageMode::Fixed(Language::Json),
        );
        assert_eq!(result.version, 7);
        assert!(result.diagnostics.is_empty() && result.missing.is_empty());
        assert!(result.coverage.is_complete());
    }
}
