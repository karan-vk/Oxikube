//! The arguments of `k8s.get_events`, read from the JSON an agent sent.
//!
//! The schema already checked types and ranges ([`validate_args`](crate::tools::validate_args));
//! this reads the values and refuses the combinations a schema subset cannot express.

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::event::EventType;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::Value;

use super::bounds::{DEFAULT_LIMIT, MAX_LIMIT};
use crate::logs::parse_since;

/// The object `involved` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum InvolvedArg {
    /// Events about the object with this uid.
    Uid(String),
    /// Events about the object of this kind and name (in `ns`, when that is given).
    Object { kind: String, name: String },
}

/// A parsed `k8s.get_events` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GetEventsArgs {
    /// Events stored in this namespace; `None`: every namespace.
    pub(super) namespace: Option<String>,
    pub(super) involved: Option<InvolvedArg>,
    /// Events last seen at or after this time.
    pub(super) since: Option<Timestamp>,
    pub(super) event_type: Option<EventType>,
    pub(super) limit: usize,
}

impl GetEventsArgs {
    /// Reads `args`; durations in `since` count back from `now`.
    ///
    /// # Errors
    ///
    /// A validation error for a bad `since`, an `involved` that names neither a uid nor a kind
    /// and name (or both), or a `limit` outside `1..=MAX_LIMIT`.
    pub(super) fn parse(args: &Value, now: Timestamp) -> OxiResult<Self> {
        Ok(Self {
            namespace: namespace(args.get("ns"))?,
            involved: involved(args.get("involved"))?,
            since: since(args.get("since"), now)?,
            event_type: event_type(args.get("type"))?,
            limit: limit(args.get("limit"))?,
        })
    }
}

fn text<'a>(value: &'a Value, what: &str) -> OxiResult<&'a str> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| OxiError::validation(format!("`{what}` must be a non-empty string")))
}

fn namespace(value: Option<&Value>) -> OxiResult<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let ns = text(value, "ns")?;
            Ok((!ns.eq_ignore_ascii_case("all") && ns != "*").then(|| ns.to_owned()))
        }
    }
}

fn involved(value: Option<&Value>) -> OxiResult<Option<InvolvedArg>> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let field = |key: &str| {
        value
            .get(key)
            .filter(|v| !v.is_null())
            .map(|v| text(v, &format!("involved.{key}")).map(str::to_owned))
            .transpose()
    };
    let (uid, kind, name) = (field("uid")?, field("kind")?, field("name")?);
    match (uid, kind, name) {
        (Some(uid), None, None) => Ok(Some(InvolvedArg::Uid(uid))),
        (None, Some(kind), Some(name)) => Ok(Some(InvolvedArg::Object { kind, name })),
        _ => Err(OxiError::validation(
            "`involved` names an object by `uid`, or by `kind` and `name` (not both, not a part)",
        )),
    }
}

fn since(value: Option<&Value>, now: Timestamp) -> OxiResult<Option<Timestamp>> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let text = text(value, "since")?;
    if let Ok(back) = parse_since(text) {
        let back = SignedDuration::try_from(back)
            .map_err(|_| OxiError::validation(format!("since {text:?} is too long")))?;
        return now
            .checked_sub(back)
            .map(Some)
            .map_err(|_| OxiError::validation(format!("since {text:?} is too long")));
    }
    text.parse::<Timestamp>().map(Some).map_err(|_| {
        OxiError::validation(format!(
            "since {text:?}: expected a duration like 15m, 2h or 1d (up to 7 days), or an RFC 3339 \
             timestamp like 2026-01-01T00:00:00Z"
        ))
    })
}

fn event_type(value: Option<&Value>) -> OxiResult<Option<EventType>> {
    match value.filter(|v| !v.is_null()).map(|v| text(v, "type")) {
        None => Ok(None),
        Some(Ok("Warning")) => Ok(Some(EventType::Warning)),
        Some(Ok("Normal")) => Ok(Some(EventType::Normal)),
        Some(Ok(other)) => Err(OxiError::validation(format!(
            "type {other:?}: expected `Warning` or `Normal`"
        ))),
        Some(Err(e)) => Err(e),
    }
}

fn limit(value: Option<&Value>) -> OxiResult<usize> {
    match value.filter(|v| !v.is_null()) {
        None => Ok(DEFAULT_LIMIT),
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=MAX_LIMIT as u64).contains(n))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| {
                OxiError::validation(format!("limit must be an integer from 1 to {MAX_LIMIT}"))
            }),
    }
}
