//! [`LogSaveScope`]: which lines `logs::Save` offers to write to a file.

use serde::{Deserialize, Serialize};

/// Which lines of a log view `logs::Save` offers to write. Both honour the view's active filter
/// ("what you see is what you export"); they differ in how far back they reach.
///
/// Serialised as `"visible"` or `"all"`, which is what a `logs::Save` tool call passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LogSaveScope {
    /// The lines on screen right now (the viewport).
    #[serde(rename = "visible")]
    Visible,
    /// Every line the view's ring buffer holds (the newest `logs.buffer_lines`).
    #[default]
    #[serde(rename = "all")]
    All,
}

impl LogSaveScope {
    /// Both scopes, in the order a dialog lists them.
    pub const ALL: [LogSaveScope; 2] = [LogSaveScope::Visible, LogSaveScope::All];

    /// `visible` or `all`.
    pub const fn label(self) -> &'static str {
        match self {
            LogSaveScope::Visible => "visible",
            LogSaveScope::All => "all",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_as_its_label_and_round_trips() {
        for scope in LogSaveScope::ALL {
            let json = serde_json::to_value(scope).unwrap();
            assert_eq!(json, serde_json::json!(scope.label()));
            assert_eq!(serde_json::from_value::<LogSaveScope>(json).unwrap(), scope);
        }
        assert!(serde_json::from_value::<LogSaveScope>(serde_json::json!("some")).is_err());
        assert_eq!(LogSaveScope::default(), LogSaveScope::All);
    }
}
