//! Problems found while loading a keymap layer.
//!
//! Loading never fails as a whole: a syntax error keeps the previous layer, a bad section or
//! binding is skipped and everything else loads. Each case is a [`KeymapDiagnostic`], logged
//! when a layer loads and kept on the [`KeymapStore`](crate::KeymapStore) for the UI to surface
//! (a toast now, the keymap editor in E21), the same channel shape as
//! `oxikube_settings::SettingsDiagnostic`.

use std::fmt;

use crate::layer::KeymapLayer;

/// What is wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeymapProblem {
    /// The file is not valid JSON with comments, or not a list of sections. The previous keymap
    /// of that layer stays in effect.
    InvalidFile {
        /// Parser message with line and column.
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
    /// What is wrong.
    pub problem: KeymapProblem,
}

impl KeymapDiagnostic {
    pub(crate) fn file(layer: KeymapLayer, message: String) -> Self {
        Self {
            layer,
            section: None,
            keystrokes: None,
            problem: KeymapProblem::InvalidFile { message },
        }
    }

    pub(crate) fn section(layer: KeymapLayer, section: usize, problem: KeymapProblem) -> Self {
        Self {
            layer,
            section: Some(section),
            keystrokes: None,
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
            problem,
        }
    }
}

impl fmt::Display for KeymapProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFile { message } => write!(f, "is invalid: {message}"),
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.layer)?;
        if let Some(section) = self.section {
            write!(f, ", section {}", section + 1)?;
        }
        if let Some(keystrokes) = &self.keystrokes {
            write!(f, ", `{keystrokes}`")?;
        }
        match &self.problem {
            KeymapProblem::InvalidFile { .. } => write!(f, " {}", self.problem),
            problem => write!(f, ": {problem}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_layer_section_and_key() {
        let d = KeymapDiagnostic::binding(
            KeymapLayer::User,
            1,
            "cmd-k",
            KeymapProblem::UnknownAction {
                name: "x::Y".into(),
            },
        );
        assert_eq!(
            d.to_string(),
            "keymap.json, section 2, `cmd-k`: unknown action `x::Y`"
        );
        let d = KeymapDiagnostic::file(KeymapLayer::User, "line 3".into());
        assert_eq!(d.to_string(), "keymap.json is invalid: line 3");
    }
}
