use jiff::Timestamp;
use oxikube_app::{RepoActivity, RepoRow, RepoSnapshot, RepoStatus};

use super::entry;
use crate::repos::{ReposModel, Tone};

fn at(secs: i64) -> Timestamp {
    Timestamp::from_second(secs).unwrap()
}

fn snapshot(status: RepoStatus) -> RepoSnapshot {
    RepoSnapshot {
        cli_available: true,
        loaded: true,
        rows: vec![RepoRow {
            entry: entry("bitnami"),
            status,
        }],
        ..RepoSnapshot::default()
    }
}

fn status_of(status: RepoStatus, now: i64) -> (String, Tone) {
    let model = ReposModel::build(&snapshot(status), at(now));
    (model.rows[0].status.clone(), model.rows[0].tone)
}

#[test]
fn statuses_read_in_words() {
    assert_eq!(
        status_of(RepoStatus::Unknown, 0),
        ("Not refreshed in this session".into(), Tone::Muted)
    );
    assert_eq!(
        status_of(RepoStatus::Updating, 0),
        ("Updating...".into(), Tone::Pending)
    );
    let updated = |ago: i64| status_of(RepoStatus::Updated(at(100_000)), 100_000 + ago).0;
    assert_eq!(updated(5), "Updated just now");
    assert_eq!(updated(300), "Updated 5 min ago");
    assert_eq!(updated(7_300), "Updated 2 h ago");
    assert_eq!(updated(3 * 86_400 + 10), "Updated 3 d ago");
    let (text, tone) = status_of(RepoStatus::Failed("401".into()), 0);
    assert_eq!((text.as_str(), tone), ("Failed: 401", Tone::Error));
}

#[test]
fn progress_names_what_is_running() {
    let mut snap = snapshot(RepoStatus::Unknown);
    snap.activity = RepoActivity::Updating(vec!["a".into(), "b".into()]);
    let model = ReposModel::build(&snap, at(0));
    assert!(model.busy);
    assert_eq!(
        model.progress.as_deref(),
        Some("Updating 2 repositories...")
    );
    snap.activity = RepoActivity::Adding("x".into());
    assert!(
        ReposModel::build(&snap, at(0))
            .progress
            .unwrap()
            .contains("Adding x")
    );
    snap.activity = RepoActivity::Idle;
    assert!(!ReposModel::build(&snap, at(0)).busy);
}

#[test]
fn suggestions_exclude_what_is_configured() {
    let model = ReposModel::build(&snapshot(RepoStatus::Unknown), at(0));
    assert!(model.suggestions.iter().all(|s| s.name != "bitnami"));
    assert!(!model.suggestions.is_empty());
}
