//! The model and the status wording: plain Rust.

use oxikube_ports::{SourceState, UserSource};

use super::super::model::{LoadState, SourcesModel, status_text};
use super::super::test_support::{broken, found, row};

#[test]
fn a_new_model_is_loading_until_rows_arrive() {
    let mut model = SourcesModel::new();
    assert_eq!(model.load_state(), &LoadState::Loading);
    assert_eq!(model.summary(), "0 sources");
    model.set_rows(vec![found(UserSource::file("/a"), 1)]);
    assert_eq!(model.load_state(), &LoadState::Ready);
    model.set_failed("boom".into());
    assert_eq!(model.load_state(), &LoadState::Failed("boom".into()));
}

#[test]
fn the_summary_counts_sources_and_problems() {
    let mut model = SourcesModel::new();
    model.set_rows(vec![
        found(UserSource::file("/a"), 2),
        broken(
            UserSource::file("/b"),
            SourceState::Invalid,
            "Not a valid kubeconfig",
        ),
        broken(
            UserSource::file("/c"),
            SourceState::Missing,
            "File not found",
        ),
        row(
            UserSource::file("/d"),
            Some(SourceState::Blank),
            0,
            Some("File is empty"),
        ),
    ]);
    assert_eq!(
        model.error_count(),
        2,
        "a blank file is a note, not an error"
    );
    assert_eq!(model.summary(), "4 sources, 2 with a problem");
    model.set_rows(vec![found(UserSource::file("/a"), 1)]);
    assert_eq!(model.summary(), "1 source");
}

#[test]
fn status_text_reads_like_a_sentence() {
    let at = |state, contexts, message| {
        status_text(&row(UserSource::file("/a"), state, contexts, message))
    };
    assert_eq!(at(None, 0, None), "Not read yet");
    assert_eq!(at(Some(SourceState::Found), 1, None), "1 context");
    assert_eq!(at(Some(SourceState::Found), 3, None), "3 contexts");
    assert_eq!(
        at(
            Some(SourceState::Found),
            4,
            Some("1 of 5 files skipped: x.yaml: not a valid kubeconfig")
        ),
        "4 contexts, 1 of 5 files skipped: x.yaml: not a valid kubeconfig"
    );
    assert_eq!(
        at(Some(SourceState::Missing), 0, Some("File not found")),
        "File not found"
    );
    assert_eq!(
        at(Some(SourceState::Invalid), 0, None),
        "Not a valid kubeconfig"
    );
}
