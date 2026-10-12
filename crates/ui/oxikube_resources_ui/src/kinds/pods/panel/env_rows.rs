//! The environment rows of the pod panel: a resolved variable as a [`Row::Env`], its value as an
//! [`EnvCell`] and a missing reference as a chip.

use oxikube_app::columns::Tone;
use oxikube_domain::view::{EnvRow, EnvValue, Missing, Origin, SecretValue};

use super::model::{EnvLine, Row};

/// What an environment row shows as its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvCell {
    /// The value as it is.
    Plain(String),
    /// A Secret value: masked until the user reveals it. Held in memory only.
    Secret(SecretValue),
    /// No value, and why, in a few words.
    Chip {
        /// `Secret web-secret not found`.
        text: String,
        /// Amber for a problem, neutral for an `optional` reference.
        tone: Tone,
    },
}

pub(super) fn env_row(container: usize, row: EnvRow) -> Row {
    let origin = origin_text(&row.origin);
    let cell = match row.value {
        EnvValue::Plain(text) => EnvCell::Plain(text),
        EnvValue::Secret(secret) => EnvCell::Secret(secret),
        EnvValue::Missing(missing) => missing_chip(&missing),
    };
    Row::Env(EnvLine {
        key: (container, row.name.clone()),
        name: row.name,
        cell,
        origin,
    })
}

fn origin_text(origin: &Origin) -> String {
    match origin {
        Origin::Literal => String::new(),
        Origin::Source { kind, name, key } if &**key == "*" => format!("{} {name}", kind.as_str()),
        Origin::Source { kind, name, key } => format!("{} {name} · {key}", kind.as_str()),
        Origin::Field(path) => format!("fieldRef {path}"),
        Origin::Resource(resource) => format!("resourceFieldRef {resource}"),
    }
}

fn missing_chip(missing: &Missing) -> EnvCell {
    let (text, optional) = match missing {
        Missing::Object {
            kind,
            name,
            optional,
        } => (format!("{} {name} not found", kind.as_str()), *optional),
        Missing::Key {
            kind,
            name,
            key,
            optional,
        } => (
            format!("key {key} not found in {} {name}", kind.as_str()),
            *optional,
        ),
        Missing::Unavailable { kind, name, reason } => {
            (format!("{} {name}: {reason}", kind.as_str()), false)
        }
        Missing::Unset => ("not set".to_owned(), true),
    };
    let text = if optional && !matches!(missing, Missing::Unset) {
        format!("{text} (optional)")
    } else {
        text
    };
    EnvCell::Chip {
        text,
        tone: if optional { Tone::Neutral } else { Tone::Warn },
    }
}
