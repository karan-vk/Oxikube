//! Redacting what `helm` prints and mapping its failures onto the error taxonomy.
//!
//! Values reach `helm` on stdin only (`--values=-`), so no command line carries them. What
//! `helm` prints back (progress lines, stderr) can still echo them: every line is passed
//! through [`Redactor`], which blanks the string values the request supplied and then applies
//! the shared [`redact`] patterns (tokens, PEM blocks, `password:` fields, URL user-info).

use std::time::Duration;

use oxikube_domain::OxiError;
use oxikube_domain::helm::{HelmError, Values};
use oxikube_domain::redact::{MARKER, redact};
use serde_json::Value;

/// Shortest supplied value that is blanked: shorter ones (`on`, `1`, `v2`) are too common to
/// be secrets and would garble every line.
const MIN_SECRET_LEN: usize = 4;

/// Longest error summary kept, in characters.
const SUMMARY_MAX: usize = 600;

/// Blanks the values of one request, then applies the shared redaction patterns.
#[derive(Debug, Clone, Default)]
pub(crate) struct Redactor {
    /// Supplied string values, longest first (so a value containing another is blanked whole).
    needles: Vec<String>,
}

impl Redactor {
    /// Only the shared patterns.
    pub(crate) fn none() -> Self {
        Self::default()
    }

    /// The shared patterns plus every string (and number) leaf of `values`.
    pub(crate) fn for_values(values: &Values) -> Self {
        let mut needles = Vec::new();
        collect(values.as_json(), &mut needles);
        needles.retain(|n| n.chars().count() >= MIN_SECRET_LEN);
        needles.sort_by_key(|n| std::cmp::Reverse(n.len()));
        needles.dedup();
        Self { needles }
    }

    /// `text` with every supplied value and secret-looking substring replaced by the marker.
    pub(crate) fn apply(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for needle in &self.needles {
            if out.contains(needle.as_str()) {
                out = out.replace(needle.as_str(), MARKER);
            }
        }
        redact(&out).into_owned()
    }
}

fn collect(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Number(n) => out.push(n.to_string()),
        Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
        Value::Object(map) => map.values().for_each(|v| collect(v, out)),
        Value::Bool(_) | Value::Null => {}
    }
}

/// Why a `helm` invocation failed, before it is mapped for the caller.
#[derive(Debug)]
pub(crate) enum CliFailure {
    /// The process ran and exited unsuccessfully.
    Exit {
        /// Exit code, `None` when killed by a signal.
        code: Option<i32>,
        /// The redacted [`summary`] of stderr.
        summary: String,
    },
    /// It did not finish in time and was killed.
    Timeout(Duration),
    /// It could not be run, or its pipes failed.
    Other(OxiError),
}

impl CliFailure {
    /// `true` when the failure's summary contains `needle` (case-insensitive).
    pub(crate) fn says(&self, needle: &str) -> bool {
        matches!(self, Self::Exit { summary, .. } if summary.to_lowercase().contains(needle))
    }

    /// The port error; `release` names the release the command was about, for messages.
    pub(crate) fn into_error(self, release: Option<&str>) -> OxiError {
        match self {
            Self::Exit { code, summary } => classify(code, &summary, release),
            Self::Timeout(after) => HelmError::Timeout(format!("{}s", after.as_secs())).into(),
            Self::Other(error) => error,
        }
    }
}

/// A short summary of `stderr`: its `Error:` lines (Helm's own report), else its last lines;
/// warnings and log noise dropped, redacted and capped.
pub(crate) fn summary(stderr: &str, redactor: &Redactor) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let errors: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("Error:").map(str::trim))
        .collect();
    let picked = if errors.is_empty() {
        let noise = |l: &&str| l.starts_with("WARNING:") || l.starts_with("level=");
        let kept: Vec<&str> = lines.iter().copied().filter(|l| !noise(l)).collect();
        let tail = if kept.is_empty() {
            &lines[..]
        } else {
            &kept[..]
        };
        tail[tail.len().saturating_sub(3)..].join("; ")
    } else {
        errors.join("; ")
    };
    let redacted = redactor.apply(&picked);
    match redacted.char_indices().nth(SUMMARY_MAX) {
        Some((cut, _)) => format!("{}…", &redacted[..cut]),
        None => redacted,
    }
}

/// Maps a failed run onto the port taxonomy by what Helm said (patterns verified against the
/// recorded stderr of Helm 3.10 and 4.3 in `tests/fixtures/`).
pub(crate) fn classify(code: Option<i32>, summary: &str, release: Option<&str>) -> OxiError {
    let lower = summary.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    if has(&["release: not found", "release not loaded"]) {
        let name = release.map_or_else(|| summary.to_owned(), str::to_owned);
        HelmError::ReleaseNotFound(name).into()
    } else if has(&[
        "no chart version found",
        "no chart name found",
        "chart not found",
    ]) || (lower.contains("not found in") && lower.contains("index"))
        || (has(&["no repo named", "repo not found", "path \""]) && lower.contains("not found"))
    {
        OxiError::not_found(format!("chart not found: {summary}"))
    } else if has(&["cannot re-use a name", "cannot reuse a name"]) {
        OxiError::conflict(format!("the release name is already in use: {summary}"))
    } else if has(&["forbidden"]) {
        HelmError::Forbidden(summary.to_owned()).into()
    } else if has(&["unauthorized", "you must be logged in"]) {
        OxiError::auth(summary.to_owned(), false)
    } else if lower.contains("context \"") && lower.contains("does not exist") {
        OxiError::not_found(format!("kubeconfig context not found: {summary}"))
    } else if has(&[
        "cluster unreachable",
        "connection refused",
        "no such host",
        "i/o timeout",
        "dial tcp",
    ]) {
        OxiError::network(format!("the cluster is unreachable: {summary}"))
    } else if has(&["unknown flag", "unknown shorthand flag", "unknown command"]) {
        OxiError::unsupported(format!(
            "this helm version does not support the command: {summary}"
        ))
    } else {
        HelmError::CliFailed {
            exit_code: code,
            stderr_summary: summary.to_owned(),
        }
        .into()
    }
}
