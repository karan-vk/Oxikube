//! The `audit` setting (E19-S08): how long the audit log keeps its records.
//!
//! ```json
//! "audit": { "retention_days": 90 }
//! ```
//!
//! `retention_days` is the age after which the audit log drops a record (`0` keeps everything;
//! values above [`MAX_RETENTION_DAYS`] are clamped with a warning). The log is pruned when the
//! app starts, once a day after that, and at once when the value changes, so an edit applies
//! without a restart. The setting is global: the audit log is one file for every cluster.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::settings::Settings;

/// The default of `audit.retention_days`, matching `default.json`.
pub const DEFAULT_RETENTION_DAYS: u32 = 90;
/// The largest `audit.retention_days` (ten years).
pub const MAX_RETENTION_DAYS: u32 = 3650;

/// What one settings layer says about the audit log.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AuditSettingsContent {
    /// Days the audit log keeps a record before it is pruned (0 to 3650; default 90). 0 keeps
    /// every record. Applies at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_days: Option<u32>,
}

/// The resolved `audit` setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditSettings {
    /// Days a record is kept; `0` keeps everything. Already clamped to [`MAX_RETENTION_DAYS`].
    pub retention_days: u32,
}

impl Settings for AuditSettings {
    const KEY: Option<&'static str> = Some("audit");
    type Content = AuditSettingsContent;

    fn from_content(content: AuditSettingsContent) -> Self {
        let requested = content.retention_days.unwrap_or(DEFAULT_RETENTION_DAYS);
        if requested > MAX_RETENTION_DAYS {
            tracing::warn!(
                requested,
                "audit.retention_days is out of range (0 to {MAX_RETENTION_DAYS}): clamped"
            );
        }
        Self {
            retention_days: requested.min(MAX_RETENTION_DAYS),
        }
    }
}

crate::register_settings!(AuditSettings);

#[cfg(test)]
mod tests;
