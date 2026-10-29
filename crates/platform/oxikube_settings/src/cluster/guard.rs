//! The `guard` block (E19-S04): defaults of the safety posture.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Guard defaults, as one settings layer writes them.
///
/// ```json
/// "guard": { "default_read_only": true }
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GuardContent {
    /// Start every cluster that has no explicit `read_only` in read-only mode. Toggling a
    /// cluster in the app writes `clusters.<id>.read_only`, which then wins. Default `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_read_only: Option<bool>,
}
