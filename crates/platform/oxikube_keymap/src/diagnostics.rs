//! Problems found while loading a keymap layer.
//!
//! Loading never fails as a whole: a syntax error keeps the previous layer, a bad section or
//! binding is skipped and everything else loads. Each case is a [`KeymapDiagnostic`], logged
//! when a layer loads and kept on the [`KeymapStore`](crate::KeymapStore) for the UI to surface
//! (a toast now, the keymap editor in E21), the same channel shape as
//! `oxikube_settings::SettingsDiagnostic`.

use std::fmt;

use crate::layer::KeymapLayer;
use crate::lines::SourceLines;

/// What is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeymapProblem {
    /// The file is not valid JSON with comments, or not a list of sections. The previous keymap
    /// of that layer stays in effect.
    InvalidFile {
        /// Parser message with line and column.
        message: String,
    },
    /// The file exists but could not be read (not UTF-8, permission denied, a directory). The
    /// defaults stay in effect and the file is still watched, so fixing it reloads it.
    Unreadable {
        /// The I/O error.
        message: String,
    },
    /// A list entry that is not a valid section (wrong type, unknown field).
    InvalidSection {
        /// Deserialiser message.
        message: String,
    },
    /// A `context` that is not a valid key-context expression. The whole section is skipped.
    InvalidContext {
        /// The context text.
        context: String,
        /// Parser message.
        message: String,
    },
    /// A key that is not a sequence of valid keystrokes.
    InvalidKeystrokes {
        /// Parser message naming the keystroke.
        message: String,
    },
    /// A binding value that is not a name, `[name, data]` or `null`.
    InvalidBinding {
        /// What was found instead.
        message: String,
    },
    /// An action name no crate has registered.
    UnknownAction {
        /// The name as written.
        name: String,
    },
    /// The action exists but rejected the data given for it.
    InvalidActionData {
        /// The action name.
        name: String,
        /// Deserialiser message.
        message: String,
    },
}

/// One problem and where it was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapDiagnostic {
    /// The layer the problem is in.
    pub layer: KeymapLayer,
    /// Index of the section in the layer's list (`None` for a whole-file problem).
    pub section: Option<usize>,
    /// The binding's keystrokes as written (`None` for a section or file problem).
    pub keystrokes: Option<String>,
    /// The 1-based line in the file the problem is on: the binding's, the section's, or the
    /// parser's for a syntax error. `None` for the embedded layers and an unreadable file.
    pub line: Option<usize>,
    /// What is wrong.
    pub problem: KeymapProblem,
}

impl KeymapDiagnostic {
    pub(crate) fn file(layer: KeymapLayer, line: Option<usize>, message: String) -> Self {
        Self {
            layer,
            section: None,
            keystrokes: None,
            line,
            problem: KeymapProblem::InvalidFile { message },
        }
    }

    pub(crate) fn unreadable(layer: KeymapLayer, message: String) -> Self {
        Self {
            layer,
            section: None,
            keystrokes: None,
            line: None,
            problem: KeymapProblem::Unreadable { message },
        }
    }

    pub(crate) fn section(layer: KeymapLayer, section: usize, problem: KeymapProblem) -> Self {
        Self {
            layer,
            section: Some(section),
            keystrokes: None,
            line: None,
            problem,
        }
    }

    pub(crate) fn binding(
        layer: KeymapLayer,
        section: usize,
        keystrokes: &str,
        problem: KeymapProblem,
    ) -> Self {
        Self {
            layer,
            section: Some(section),
            keystrokes: Some(keystrokes.to_owned()),
            line: None,
            problem,
        }
    }

    /// Fill in the line from the file's [`SourceLines`], when this problem has none yet: the
    /// binding's line, else the `context`'s for a bad context, else the section's.
    pub(crate) fn locate(&mut self, lines: &SourceLines) {
        if self.line.is_some() {
            return;
        }
        let Some(section) = self.section else { return };
        let exact = match (&self.keystrokes, &self.problem) {
            (Some(keystrokes), _) => lines.binding(section, keystrokes),
            (None, KeymapProblem::InvalidContext { .. }) => lines.context(section),
            (None, _) => None,
        };
        self.line = exact.or_else(|| lines.section(section));
    }
}

impl fmt::Display for KeymapProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFile { message } => write!(f, "invalid file: {message}"),
            Self::Unreadable { message } => write!(f, "could not be read: {message}"),
            Self::InvalidSection { message } => write!(f, "section is invalid: {message}"),
            Self::InvalidContext { context, message } => {
                write!(f, "invalid context `{context}`: {message}")
            }
            Self::InvalidKeystrokes { message } => f.write_str(message),
            Self::InvalidBinding { message } => write!(f, "invalid binding: {message}"),
            Self::UnknownAction { name } => write!(f, "unknown action `{name}`"),
            Self::InvalidActionData { name, message } => {
                write!(f, "invalid data for action `{name}`: {message}")
            }
        }
    }
}

impl fmt::Display for KeymapDiagnostic {
    /// `keymap.json:12: unknown action `x::Y` (binding `cmd-k`)`. The line is left out when it is
    /// not known (the embedded layers, an unreadable file), and then the section stands in for it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.layer)?;
        match (self.line, self.section) {
            (Some(line), _) => write!(f, ":{line}")?,
            (None, Some(section)) => write!(f, ", section {}", section + 1)?,
            (None, None) => {}
        }
        write!(f, ": {}", self.problem)?;
        if let Some(keystrokes) = &self.keystrokes {
            write!(f, " (binding `{keystrokes}`)")?;
        }
        Ok(())
    }
}

/// What the keymap tells the rest of the app when the problems with the user's `keymap.json`
/// change: the list that is now current, empty when the file was fixed. Platform code cannot show
/// a toast, so the binary subscribes ([`crate::subscribe_diagnostics`]) and does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeymapDiagnosticsEvent {
    /// Every problem found in the user's file by the load that raised the event, in file order.
    pub diagnostics: Vec<KeymapDiagnostic>,
}

/// How many problems [`KeymapDiagnosticsEvent::message`] lists before "and N more".
pub const MESSAGE_LINES: usize = 5;

impl KeymapDiagnosticsEvent {
    /// Whether the file is fine now (the event only clears an earlier notification).
    pub fn is_clear(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// The one notification for all the problems: a sentence saying what still works, then a line
    /// per problem (`keymap.json:12: ...`), at most [`MESSAGE_LINES`] of them.
    pub fn message(&self) -> String {
        let count = self.diagnostics.len();
        let whole_file = self.diagnostics.iter().any(|d| {
            matches!(
                d.problem,
                KeymapProblem::InvalidFile { .. } | KeymapProblem::Unreadable { .. }
            )
        });
        let mut out = if whole_file {
            "keymap.json could not be loaded; the previous keymap stays in effect.".to_owned()
        } else if count == 1 {
            "1 problem in keymap.json; the other bindings were applied.".to_owned()
        } else {
            format!("{count} problems in keymap.json; the other bindings were applied.")
        };
        for diagnostic in self.diagnostics.iter().take(MESSAGE_LINES) {
            out.push('\n');
            out.push_str(&diagnostic.to_string());
        }
        if count > MESSAGE_LINES {
            out.push_str(&format!("\nand {} more", count - MESSAGE_LINES));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unknown(line: Option<usize>) -> KeymapDiagnostic {
        let mut d = KeymapDiagnostic::binding(
            KeymapLayer::User,
            1,
            "cmd-k",
            KeymapProblem::UnknownAction {
                name: "x::Y".into(),
            },
        );
        d.line = line;
        d
    }

    #[test]
    fn display_names_the_file_line_and_key() {
        assert_eq!(
            unknown(Some(12)).to_string(),
            "keymap.json:12: unknown action `x::Y` (binding `cmd-k`)"
        );
        // Without a line the section stands in.
        assert_eq!(
            unknown(None).to_string(),
            "keymap.json, section 2: unknown action `x::Y` (binding `cmd-k`)"
        );
        let d = KeymapDiagnostic::file(KeymapLayer::User, Some(3), "expected value".into());
        assert_eq!(d.to_string(), "keymap.json:3: invalid file: expected value");
        let d = KeymapDiagnostic::unreadable(
            KeymapLayer::User,
            "stream did not contain valid UTF-8".into(),
        );
        assert_eq!(
            d.to_string(),
            "keymap.json: could not be read: stream did not contain valid UTF-8"
        );
    }

    #[test]
    fn locate_prefers_the_binding_then_the_context_then_the_section() {
        let lines = SourceLines::scan(
            "[\n{\n\"context\": \"Bad ((\",\n\"bindings\": {\n\"cmd-k\": \"x::Y\"}}\n]",
        );
        let mut binding = KeymapDiagnostic::binding(
            KeymapLayer::User,
            0,
            "cmd-k",
            KeymapProblem::UnknownAction {
                name: "x::Y".into(),
            },
        );
        binding.locate(&lines);
        assert_eq!(binding.line, Some(5));
        let mut context = KeymapDiagnostic::section(
            KeymapLayer::User,
            0,
            KeymapProblem::InvalidContext {
                context: "Bad ((".into(),
                message: "x".into(),
            },
        );
        context.locate(&lines);
        assert_eq!(context.line, Some(3));
        let mut section = KeymapDiagnostic::section(
            KeymapLayer::User,
            0,
            KeymapProblem::InvalidSection {
                message: "x".into(),
            },
        );
        section.locate(&lines);
        assert_eq!(section.line, Some(2));
        // A binding the scan did not see falls back to its section.
        let mut missing = KeymapDiagnostic::binding(
            KeymapLayer::User,
            0,
            "cmd-z",
            KeymapProblem::UnknownAction {
                name: "x::Y".into(),
            },
        );
        missing.locate(&lines);
        assert_eq!(missing.line, Some(2));
    }

    #[test]
    fn the_notification_message_summarises_with_lines() {
        let event = KeymapDiagnosticsEvent {
            diagnostics: vec![unknown(Some(12)), {
                let mut d = KeymapDiagnostic::binding(
                    KeymapLayer::User,
                    2,
                    "ctrl-bogus-x",
                    KeymapProblem::InvalidKeystrokes {
                        message: "invalid keystroke `ctrl-bogus-x`".into(),
                    },
                );
                d.line = Some(20);
                d
            }],
        };
        assert_eq!(
            event.message(),
            "2 problems in keymap.json; the other bindings were applied.\n\
             keymap.json:12: unknown action `x::Y` (binding `cmd-k`)\n\
             keymap.json:20: invalid keystroke `ctrl-bogus-x` (binding `ctrl-bogus-x`)"
        );
    }

    #[test]
    fn the_message_caps_its_lines_and_has_wording_for_a_broken_file() {
        let many = KeymapDiagnosticsEvent {
            diagnostics: (1..=8).map(|n| unknown(Some(n))).collect(),
        };
        let message = many.message();
        assert_eq!(message.lines().count(), 1 + MESSAGE_LINES + 1);
        assert!(message.ends_with("and 3 more"), "{message}");
        assert!(message.starts_with("8 problems"), "{message}");

        let broken = KeymapDiagnosticsEvent {
            diagnostics: vec![KeymapDiagnostic::file(
                KeymapLayer::User,
                Some(3),
                "expected value at line 3 column 1".into(),
            )],
        };
        assert_eq!(
            broken.message(),
            "keymap.json could not be loaded; the previous keymap stays in effect.\n\
             keymap.json:3: invalid file: expected value at line 3 column 1"
        );
        let one = KeymapDiagnosticsEvent {
            diagnostics: vec![unknown(Some(1))],
        };
        assert!(one.message().starts_with("1 problem in keymap.json;"));
        assert!(
            KeymapDiagnosticsEvent {
                diagnostics: vec![]
            }
            .is_clear()
        );
    }
}
