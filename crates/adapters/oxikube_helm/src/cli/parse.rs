//! Parsers for what `helm` prints: the `-o json` shapes of `list`, `history`, `get values`,
//! `repo list` and `search repo`, plus `get notes`. Shapes checked against the recorded output
//! of Helm 3.10 and 4.3 (`tests/fixtures/helm-*`); unknown fields are ignored and unknown
//! statuses read as `Unknown`, so a newer Helm does not break the list.

use std::str::FromStr as _;

use jiff::Timestamp;
use oxikube_domain::helm::{
    ChartHit, ChartRef, HelmError, Release, ReleaseRef, ReleaseStatus, RepoEntry, Revision, Values,
};
use oxikube_domain::ids::ClusterId;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn json<T: DeserializeOwned>(what: &str, out: &str) -> Result<T, HelmError> {
    serde_json::from_str(out.trim()).map_err(|e| HelmError::Parse(format!("helm {what}: {e}")))
}

/// `helm list -o json` prints the revision as a string, `helm history` as a number.
fn revision(value: &Value) -> u32 {
    match value {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
        Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn non_empty(s: String) -> Option<String> {
    (!s.trim().is_empty()).then_some(s)
}

/// Helm's timestamps: RFC 3339 in `history` (`2026-10-21T15:58:00.362923+09:00`), Go's
/// `time.String` in `list` (`2026-10-21 15:58:01.274296 +0900 JST`). Unreadable ones are the
/// epoch rather than an error: one odd row must not hide the list.
pub(crate) fn timestamp(s: &str) -> Timestamp {
    if let Ok(ts) = Timestamp::from_str(s.trim()) {
        return ts;
    }
    let mut parts = s.split_whitespace();
    let (Some(date), Some(time), Some(offset)) = (parts.next(), parts.next(), parts.next()) else {
        return Timestamp::UNIX_EPOCH;
    };
    let offset = match offset.len() {
        5 => format!("{}:{}", &offset[..3], &offset[3..]),
        _ => offset.to_owned(),
    };
    Timestamp::from_str(&format!("{date}T{time}{offset}")).unwrap_or(Timestamp::UNIX_EPOCH)
}

/// Splits Helm's chart column (`kube-prometheus-stack-51.2.0`, `cert-manager-v1.14.0`) at the
/// first `-` followed by an `x.y` version (digit-led, or `v` and a digit). Without one the whole
/// text is the name.
pub(crate) fn chart(column: &str) -> ChartRef {
    let split = column.match_indices('-').map(|(i, _)| i).find(|&i| {
        let rest = &column[i + 1..];
        let digits = rest.strip_prefix('v').unwrap_or(rest);
        digits.starts_with(|c: char| c.is_ascii_digit()) && rest.contains('.')
    });
    match split {
        Some(i) => ChartRef::new(&column[..i], &column[i + 1..]),
        None => ChartRef::new(column, ""),
    }
}

#[derive(Deserialize)]
struct ListEntry {
    name: String,
    namespace: String,
    revision: Value,
    #[serde(default)]
    updated: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    chart: String,
    #[serde(default)]
    app_version: String,
}

/// `helm list -o json`: list-shaped [`Release`]s of `cluster`.
pub(crate) fn releases(cluster: &ClusterId, out: &str) -> Result<Vec<Release>, HelmError> {
    let entries: Vec<ListEntry> = json("list", out)?;
    Ok(entries
        .into_iter()
        .map(|e| {
            let release = ReleaseRef::new(cluster.clone(), e.namespace, e.name);
            let status = ReleaseStatus::parse(&e.status);
            let mut row = Release::summary(
                release,
                chart(&e.chart),
                revision(&e.revision),
                status,
                timestamp(&e.updated),
            );
            row.app_version = non_empty(e.app_version);
            row
        })
        .collect())
}

#[derive(Deserialize)]
struct HistoryEntry {
    revision: Value,
    #[serde(default)]
    updated: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    chart: String,
    #[serde(default)]
    app_version: String,
    #[serde(default)]
    description: String,
}

/// `helm history -o json` (oldest first): the [`Revision`]s, newest first.
pub(crate) fn history(out: &str) -> Result<Vec<Revision>, HelmError> {
    let entries: Vec<HistoryEntry> = json("history", out)?;
    let mut revisions: Vec<Revision> = entries
        .into_iter()
        .map(|e| Revision {
            revision: revision(&e.revision),
            status: ReleaseStatus::parse(&e.status),
            chart: chart(&e.chart),
            app_version: non_empty(e.app_version),
            updated: timestamp(&e.updated),
            description: e.description,
        })
        .collect();
    revisions.sort_by_key(|r| std::cmp::Reverse(r.revision));
    Ok(revisions)
}

/// `helm get values -o json` (`null` when the user supplied none).
pub(crate) fn values(out: &str) -> Result<Values, HelmError> {
    json::<Value>("get values", out).map(Values::from_json)
}

/// `helm get notes`: the text after Helm's `NOTES:` header, `None` when empty.
pub(crate) fn notes(out: &str) -> Option<String> {
    let body = out.strip_prefix("NOTES:").unwrap_or(out);
    let body = body.strip_prefix('\n').unwrap_or(body);
    (!body.trim().is_empty()).then(|| body.to_owned())
}

#[derive(Deserialize)]
struct RepoRow {
    name: String,
    url: String,
}

/// `helm repo list -o json`.
pub(crate) fn repos(out: &str) -> Result<Vec<RepoEntry>, HelmError> {
    let rows: Vec<RepoRow> = json("repo list", out)?;
    Ok(rows
        .into_iter()
        .map(|r| RepoEntry {
            name: r.name,
            url: r.url,
        })
        .collect())
}

#[derive(Deserialize)]
struct SearchRow {
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    app_version: String,
    #[serde(default)]
    description: String,
}

/// `helm search repo -o json`: names are `repo/chart`.
pub(crate) fn hits(out: &str) -> Result<Vec<ChartHit>, HelmError> {
    let rows: Vec<SearchRow> = json("search repo", out)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let chart = match r.name.split_once('/') {
                Some((repo, name)) => ChartRef::new(name, r.version).in_repo(repo),
                None => ChartRef::new(r.name, r.version),
            };
            ChartHit {
                chart,
                app_version: non_empty(r.app_version),
                description: r.description,
            }
        })
        .collect())
}
