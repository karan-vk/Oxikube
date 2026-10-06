//! `status.conditions` as table rows.

use super::status::cut;
use jiff::Timestamp;
use oxikube_app::columns::Tone;
use oxikube_domain::Resource;

/// Most conditions listed.
const MAX_CONDITIONS: usize = 50;
/// Longest message kept, in characters.
const MAX_MESSAGE: usize = 400;

/// One condition: type, status, reason, message and when it last changed.
#[derive(Debug, Clone, PartialEq)]
pub struct ConditionRow {
    /// `type`, for example `Ready`.
    pub kind: String,
    /// `status`: `True`, `False` or `Unknown`.
    pub status: String,
    /// `reason`, when set.
    pub reason: String,
    /// `message`, cut at 400 characters.
    pub message: String,
    /// `lastTransitionTime` (or, for conditions that only have one, `lastUpdateTime`).
    pub transition: Option<Timestamp>,
    /// Whether the status is the healthy one for this type.
    pub tone: Tone,
}

/// The conditions of `resource`, in the order the object lists them; empty when it has none.
pub fn conditions_of(resource: &Resource) -> Vec<ConditionRow> {
    let Some(items) = resource
        .get("/status/conditions")
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| item.as_object())
        .take(MAX_CONDITIONS)
        .map(|item| {
            let text = |field: &str| {
                item.get(field)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_owned()
            };
            let kind = text("type");
            let status = text("status");
            let message = cut(&text("message"), MAX_MESSAGE);
            let transition = ["lastTransitionTime", "lastUpdateTime"]
                .into_iter()
                .find_map(|field| item.get(field)?.as_str()?.parse::<Timestamp>().ok());
            ConditionRow {
                tone: tone(&kind, &status),
                kind,
                status,
                reason: text("reason"),
                message,
                transition,
            }
        })
        .collect()
}

/// Whether `True` is the bad answer for this condition type (`MemoryPressure`, `Failed`...).
fn negative(kind: &str) -> bool {
    const BAD: [&str; 7] = [
        "Pressure",
        "Unavailable",
        "Failure",
        "Failed",
        "Stalled",
        "Degraded",
        "Terminating",
    ];
    BAD.iter().any(|suffix| kind.ends_with(suffix))
}

/// The tone of a condition: healthy when its status is the good one for its type; `Unknown` and
/// a mismatch warn, and a missing `Ready` or `Available` is an error.
fn tone(kind: &str, status: &str) -> Tone {
    let healthy = match status {
        "True" => !negative(kind),
        "False" => negative(kind),
        _ => return Tone::Warn,
    };
    match (healthy, kind) {
        (true, _) => Tone::Ok,
        (false, "Ready" | "Available") => Tone::Error,
        (false, _) => Tone::Warn,
    }
}
