//! Conversion between YAML and JSON in the editor (E10-S10): `editor::ConvertToJson` and
//! `editor::ConvertToYaml`, and the language switch (`editor::UseJson`, `editor::UseYaml`).
//!
//! A conversion reads the buffer as a snapshot (the rope is shared, so taking it costs nothing
//! on the UI thread), converts on the background executor ([`crate::json::convert`]), and then,
//! if the buffer is still at the version it was taken at, replaces the whole text as **one undo
//! step** and switches the language to the target. Anything that could not be carried over (the
//! comments JSON has no place for) is said in the notice bar in the same moment. The editor stays
//! usable while it runs; typing meanwhile makes the result stale and it is dropped.
//!
//! The language is editor state: switching it never touches the text or the undo history.

use std::time::Duration;

use gpui::{AppContext as _, Context, Window};
use oxikube_ui::editor::EditorApi as _;

use super::manifest_editor::ManifestEditor;
use super::notice::{Notice, NoticeLevel};
use crate::json::{self, Converted, Language, LanguageMode};

/// What a conversion says when the editor is read-only.
const READ_ONLY_REFUSAL: &str = "The editor is read-only: allow editing before converting.";

impl ManifestEditor {
    /// Reads the buffer as `language` from now on, whatever the content looks like
    /// (`editor::UseJson`, `editor::UseYaml`). The text is untouched.
    pub fn use_language(
        &mut self,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model.set_mode(LanguageMode::Fixed(language));
        self.editor.update(cx, |editor, cx| {
            editor.api(window, cx).set_language(language.grammar());
        });
        self.notice = None;
        self.schedule_validation(Duration::ZERO, window, cx);
        cx.notify();
    }

    /// Replaces the buffer with the same data in `to` (`editor::ConvertToJson`,
    /// `editor::ConvertToYaml`), as one undo step. See the module docs.
    pub fn convert_to(&mut self, to: Language, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.read(cx).is_read_only() {
            self.set_notice(Notice::new(NoticeLevel::Error, READ_ONLY_REFUSAL), cx);
            return;
        }
        let snapshot = self.editor.read(cx).snapshot(cx);
        let version = snapshot.version();
        let mode = self.model.mode();
        self.set_notice(
            Notice::new(NoticeLevel::Info, format!("Converting to {to}…")),
            cx,
        );
        self.conversion = Some(cx.spawn_in(window, async move |this, cx| {
            let outcome = cx
                .background_spawn(async move { run(&snapshot.to_text(), mode, to) })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.finish_conversion(version, to, outcome, window, cx);
            });
        }));
    }

    fn finish_conversion(
        &mut self,
        version: u64,
        to: Language,
        outcome: Outcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.read(cx).version() != version {
            self.set_notice(
                Notice::new(
                    NoticeLevel::Warning,
                    format!("The text changed while converting to {to}; convert again."),
                ),
                cx,
            );
            return;
        }
        // Read-only may have been switched on while the conversion ran, and `replace_text` does
        // not check it: the decision is made here, with the text and history untouched.
        if self.editor.read(cx).is_read_only() {
            self.set_notice(Notice::new(NoticeLevel::Error, READ_ONLY_REFUSAL), cx);
            return;
        }
        match outcome {
            Outcome::Already => {
                self.set_notice(Notice::new(NoticeLevel::Info, format!("Already {to}.")), cx);
            }
            Outcome::Failed(error) => {
                if let Some(span) = error.span.clone() {
                    self.editor.update(cx, |editor, cx| {
                        editor.api(window, cx).select(span);
                    });
                }
                self.set_notice(
                    Notice::new(
                        NoticeLevel::Error,
                        format!("Cannot convert to {to}: {}", error.message),
                    ),
                    cx,
                );
            }
            // Already in that form (a JSON buffer read as YAML, say): nothing to replace, so no
            // empty undo step; the language still switches.
            Outcome::Unchanged(text) => {
                self.switch_language(to, &text, window, cx);
                // Switching the mode dropped the counts and the shown version, and no edit
                // follows to bring a check: read the text as `to` now.
                self.schedule_validation(Duration::ZERO, window, cx);
                self.set_notice(
                    Notice::new(NoticeLevel::Info, format!("Already written as {to}.")),
                    cx,
                );
            }
            Outcome::Converted(converted) => {
                // The notice survives the change event the replacement emits.
                self.keep_notice = true;
                self.switch_language(to, &converted.text, window, cx);
                self.editor.update(cx, |editor, cx| {
                    editor.api(window, cx).replace_text(&converted.text);
                });
                self.notice = Some(converted_notice(to, &converted));
                cx.notify();
            }
        }
    }

    /// Reads the buffer as `to` from now on, the language first so the check that follows an
    /// edit reads the new text as what it now is. The content decides from here on (`{` first
    /// means JSON) when it agrees with `text`, the buffer's text after the conversion, so an
    /// undo that brings the old text back brings its language back too.
    fn switch_language(
        &mut self,
        to: Language,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = if Language::sniff(text) == to {
            LanguageMode::Auto
        } else {
            LanguageMode::Fixed(to)
        };
        self.model.set_mode(mode);
        self.model.adopt(to);
        self.editor.update(cx, |editor, cx| {
            editor.api(window, cx).set_language(to.grammar());
        });
    }
}

/// What a conversion came to, computed off the UI thread.
enum Outcome {
    /// The buffer is read as `to` already: nothing to do.
    Already,
    /// The text cannot be converted.
    Failed(json::ConvertError),
    /// The converted text equals the buffer's (the text, kept for the language decision).
    Unchanged(String),
    /// The converted text.
    Converted(Converted),
}

fn run(text: &str, mode: LanguageMode, to: Language) -> Outcome {
    if mode.resolve(text) == to {
        return Outcome::Already;
    }
    match json::convert(text, to) {
        Err(error) => Outcome::Failed(error),
        Ok(converted) if converted.text == text => Outcome::Unchanged(converted.text),
        Ok(converted) => Outcome::Converted(converted),
    }
}

/// The line that says what a conversion did.
fn converted_notice(to: Language, converted: &Converted) -> Notice {
    let from = to.other();
    let undo = format!("Undo brings the {from} back.");
    let (level, text) = match converted.warnings.summary() {
        Some(summary) if converted.warnings.lossy() => (
            NoticeLevel::Warning,
            format!("Converted to {to}, but {summary}. {undo}"),
        ),
        Some(summary) => (
            NoticeLevel::Info,
            format!("Converted to {to}: {summary}. {undo}"),
        ),
        None => (NoticeLevel::Info, format!("Converted to {to}. {undo}")),
    };
    Notice::new(level, text).undoable()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::Warnings;

    fn converted(warnings: Warnings) -> Converted {
        Converted {
            text: String::new(),
            to: Language::Json,
            warnings,
        }
    }

    #[test]
    fn a_lossless_conversion_says_so_briefly() {
        let notice = converted_notice(Language::Json, &converted(Warnings::default()));
        assert_eq!(notice.level, NoticeLevel::Info);
        assert_eq!(notice.text, "Converted to JSON. Undo brings the YAML back.");
        assert!(notice.undoable);
    }

    #[test]
    fn dropped_comments_are_a_warning_with_their_count() {
        let warnings = Warnings {
            comments: 3,
            ..Warnings::default()
        };
        let notice = converted_notice(Language::Json, &converted(warnings));
        assert_eq!(notice.level, NoticeLevel::Warning);
        assert_eq!(
            notice.text,
            "Converted to JSON, but 3 comments dropped. Undo brings the YAML back."
        );
    }

    #[test]
    fn a_reading_that_loses_nothing_is_only_information() {
        let warnings = Warnings {
            octal: 1,
            ..Warnings::default()
        };
        let notice = converted_notice(Language::Json, &converted(warnings));
        assert_eq!(notice.level, NoticeLevel::Info);
        assert!(notice.text.contains("1 integer read as octal"));
    }
}
