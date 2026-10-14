//! The notification keys of the `events` settings (E14-S09): which kinds and reasons notify and
//! how a burst is rate limited, resolved into the centre's
//! [`NotificationRules`](oxikube_app::notifications::NotificationRules).
//!
//! ```json
//! "events": {
//!   "watched_kinds": ["Pod", "Deployment", "Node"],
//!   "ignore_reasons": ["BackOff"],
//!   "rate_limit": { "per_object": "5m", "per_reason": 3, "group_window": "1m",
//!                   "max_raised": 5, "raise_window": "30s" }
//! }
//! ```
//!
//! Lists are trimmed and de-duplicated, empty entries dropped, and capped; a number or duration
//! out of range is clamped and one that does not parse falls back to the default, each with a
//! warning in the log, never a refusal. The rate limit is one per app (a value under
//! `clusters.<id>.events.rate_limit` is not read), the lists can be given per cluster.

use std::sync::Arc;
use std::time::Duration;

use gpui::App;
use oxikube_app::notifications::{ClusterRules, NotificationRules};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::SettingsStore;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::EventsSettings;

/// Kinds a `watched_kinds` list keeps; the rest are dropped.
pub const MAX_WATCHED_KINDS: usize = 64;
/// Reasons an `ignore_reasons` list keeps; the rest are dropped.
pub const MAX_IGNORED_REASONS: usize = 256;

const SECS_PER_DAY: u64 = 86_400;

/// The `events.rate_limit` object: what keeps a burst of Warning events from becoming spam.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RateLimitContent {
    /// A notification for the same object toasts (and reaches the desktop) at most once per this
    /// long; occurrences in between only count up its row in the centre. A number and a unit:
    /// `s`econds, `m`inutes, `h`ours or `d`ays (`5m`); 1 second to 1 day. Default `5m`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^\s*\d+\s*[smhd]\s*$"))]
    pub per_object: Option<String>,
    /// Objects of one reason in one cluster that get a notification of their own within
    /// `group_window`; the next ones fold into one group row ("12 pods: BackOff"). 1 to 100.
    /// Default 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 100))]
    pub per_reason: Option<i64>,
    /// The window `per_reason` counts in (`1m`; 1 second to 1 hour). Default `1m`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^\s*\d+\s*[smhd]\s*$"))]
    pub group_window: Option<String>,
    /// At most this many notifications toast (and reach the desktop) per `raise_window`, whatever
    /// their reasons; the others are only listed in the centre. 1 to 100. Default 5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 100))]
    pub max_raised: Option<i64>,
    /// The window `max_raised` counts in (`30s`; 1 second to 1 hour). Default `30s`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^\s*\d+\s*[smhd]\s*$"))]
    pub raise_window: Option<String>,
}

/// The resolved `events.rate_limit`: every value already parsed and within its bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimit {
    /// How often one object's notification may toast.
    pub per_object: Duration,
    /// Objects of one reason with a notification of their own before the rest fold.
    pub per_reason: usize,
    /// The window `per_reason` counts in.
    pub group_window: Duration,
    /// Notifications that may toast per `raise_window`.
    pub max_raised: usize,
    /// The window `max_raised` counts in.
    pub raise_window: Duration,
}

impl Default for RateLimit {
    /// What the shipped `default.json` says.
    fn default() -> Self {
        Self::from_content(&RateLimitContent::default())
    }
}

impl RateLimit {
    pub(super) fn from_content(content: &RateLimitContent) -> Self {
        Self {
            per_object: duration(
                "events.rate_limit.per_object",
                content.per_object.as_deref(),
                300,
                (1, SECS_PER_DAY),
            ),
            per_reason: number("events.rate_limit.per_reason", content.per_reason, 3, 100),
            group_window: duration(
                "events.rate_limit.group_window",
                content.group_window.as_deref(),
                60,
                (1, 3_600),
            ),
            max_raised: number("events.rate_limit.max_raised", content.max_raised, 5, 100),
            raise_window: duration(
                "events.rate_limit.raise_window",
                content.raise_window.as_deref(),
                30,
                (1, 3_600),
            ),
        }
    }
}

/// `value` (or `default`) within `1..=max`, with a warning when clamped.
fn number(key: &'static str, value: Option<i64>, default: usize, max: usize) -> usize {
    let Some(value) = value else {
        return default;
    };
    let clamped = value.clamp(1, max as i64);
    if clamped != value {
        tracing::warn!(
            key,
            value,
            clamped,
            "setting out of range (1 to {max}): using the nearest allowed value"
        );
    }
    clamped as usize
}

/// `5m`, `30s`, `1d` as a duration within `bounds` seconds; `default` seconds for nothing or text
/// that does not parse (with a warning).
fn duration(key: &'static str, text: Option<&str>, default: u64, bounds: (u64, u64)) -> Duration {
    let Some(text) = text else {
        return Duration::from_secs(default);
    };
    let trimmed = text.trim();
    let parsed = trimmed
        .split_at_checked(trimmed.len().saturating_sub(1))
        .and_then(|(number, unit)| {
            let number: u64 = number.trim().parse().ok()?;
            let unit = match unit {
                "s" => 1,
                "m" => 60,
                "h" => 3_600,
                "d" => SECS_PER_DAY,
                _ => return None,
            };
            number.checked_mul(unit)
        });
    match parsed {
        Some(secs) => {
            let clamped = secs.clamp(bounds.0, bounds.1);
            if clamped != secs {
                tracing::warn!(
                    key,
                    value = text,
                    "setting out of range ({}s to {}s): using the nearest allowed value",
                    bounds.0,
                    bounds.1
                );
            }
            Duration::from_secs(clamped)
        }
        None => {
            tracing::warn!(
                key,
                value = text,
                "not a duration like 30s, 5m, 1h or 1d: using the default"
            );
            Duration::from_secs(default)
        }
    }
}

/// `entries` (or `default` when unset) trimmed, without empty or repeated entries (`fold_case`
/// compares ignoring ASCII case), at most `max`.
pub(super) fn names(
    key: &'static str,
    entries: Option<&[String]>,
    default: &[&str],
    fold_case: bool,
    max: usize,
) -> Vec<Arc<str>> {
    let given: Vec<&str> = match entries {
        Some(entries) => entries.iter().map(String::as_str).collect(),
        None => default.to_vec(),
    };
    let mut out: Vec<Arc<str>> = Vec::with_capacity(given.len().min(max));
    for entry in given.into_iter().map(str::trim).filter(|e| !e.is_empty()) {
        let seen = |known: &Arc<str>| {
            if fold_case {
                known.eq_ignore_ascii_case(entry)
            } else {
                **known == *entry
            }
        };
        if out.iter().any(seen) {
            continue;
        }
        if out.len() == max {
            tracing::warn!(key, max, "too many entries: the rest are ignored");
            break;
        }
        out.push(Arc::from(entry));
    }
    out
}

impl EventsSettings {
    /// The rules the settings in `store` say: the global kinds, ignored reasons and rate limit,
    /// and each cluster whose kinds, ignored reasons or desktop switch differ (keys of `clusters`
    /// that are not cluster ids are skipped), over `base`'s other values.
    pub fn notification_rules(store: &SettingsStore, base: NotificationRules) -> NotificationRules {
        let global = store.try_get::<Self>(None).cloned().unwrap_or_default();
        let clusters = store
            .cluster_values::<Self>()
            .filter_map(|(id, s)| {
                let id = id.parse::<ClusterId>().ok()?;
                let rules = ClusterRules {
                    watched_kinds: (s.watched_kinds != global.watched_kinds)
                        .then(|| s.watched_kinds.clone()),
                    ignored_reasons: (s.ignore_reasons != global.ignore_reasons)
                        .then(|| s.ignore_reasons.clone()),
                    os_notifications: (s.os_notifications != global.os_notifications)
                        .then_some(s.os_notifications),
                };
                (rules != ClusterRules::default()).then_some((id, rules))
            })
            .collect();
        let limit = global.rate_limit;
        NotificationRules {
            watched_kinds: global.watched_kinds,
            ignored_reasons: global.ignore_reasons,
            clusters,
            object_interval: limit.per_object,
            group_threshold: limit.per_reason,
            group_window: limit.group_window,
            max_raised: limit.max_raised,
            raise_window: limit.raise_window,
            ..base
        }
    }

    /// [`notification_rules`](Self::notification_rules) from the app's store, or `base` without
    /// one.
    pub(super) fn rules_in(cx: &App, base: NotificationRules) -> NotificationRules {
        match cx.try_global::<SettingsStore>() {
            Some(store) => Self::notification_rules(store, base),
            None => base,
        }
    }
}
