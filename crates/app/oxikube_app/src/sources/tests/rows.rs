//! The rows: the list joined with how each source was read, and reloading.

use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{
    ClusterContext, ClusterSource, SourceId, SourceKind, SourceState, SourceStatus, SourcesChanged,
};
use oxikube_testkit::ClusterSourceCall;

use super::*;

fn source_of(path: &str, kind: SourceKind) -> ClusterSource {
    ClusterSource {
        id: SourceId(format!("file:{path}")),
        kind,
        label: path.into(),
        path: Some(PathBuf::from(path)),
    }
}

fn status(path: &str, state: SourceState, contexts: usize, message: Option<&str>) -> SourceStatus {
    SourceStatus {
        source: source_of(path, SourceKind::KubeconfigFile),
        state,
        contexts,
        message: message.map(Into::into),
    }
}

fn default_status(contexts: usize) -> SourceStatus {
    SourceStatus {
        source: ClusterSource {
            id: SourceId("default".into()),
            kind: SourceKind::KubeconfigFile,
            label: "Default kubeconfig".into(),
            path: Some(PathBuf::from("/home/me/.kube/config")),
        },
        state: SourceState::Found,
        contexts,
        message: None,
    }
}

fn context(name: &str, source: &str) -> ClusterContext {
    let context = ContextName::new(name);
    ClusterContext::new(
        ClusterId::new(source, &context),
        context,
        SourceId(format!("file:{source}")),
    )
}

#[test]
fn an_invalid_file_shows_its_error_while_the_other_sources_stay_found() {
    let f = Fixture::new([
        UserSource::default_source(),
        file("/work/good.yaml"),
        file("/work/broken.yaml"),
        dir("/work/configs"),
    ]);
    f.source.set_statuses([
        default_status(3),
        status("/work/good.yaml", SourceState::Found, 2, None),
        status(
            "/work/broken.yaml",
            SourceState::Invalid,
            0,
            Some("Not a valid kubeconfig"),
        ),
        SourceStatus {
            source: source_of("/work/configs", SourceKind::KubeconfigDir),
            state: SourceState::Found,
            contexts: 4,
            message: Some("1 of 5 files skipped: bad.yaml: not a valid kubeconfig".into()),
        },
    ]);

    let rows = f.run(f.service.rows()).unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].state, Some(SourceState::Found));
    assert_eq!(rows[0].contexts, 3);
    assert!(!rows[0].is_error());

    assert_eq!(
        (rows[1].state, rows[1].contexts),
        (Some(SourceState::Found), 2)
    );
    assert!(!rows[1].is_error());

    assert!(rows[2].is_error());
    assert_eq!(rows[2].message.as_deref(), Some("Not a valid kubeconfig"));
    assert_eq!(rows[2].contexts, 0);

    assert!(
        !rows[3].is_error(),
        "a folder with some good files still works"
    );
    assert_eq!(rows[3].contexts, 4);
    assert!(rows[3].message.as_deref().unwrap().starts_with("1 of 5"));
}

#[test]
fn the_default_entry_gathers_every_kubectl_tier_status() {
    let f = Fixture::with_defaults();
    let env = |path: &str, state, contexts| SourceStatus {
        source: source_of(path, SourceKind::Environment),
        state,
        contexts,
        message: None,
    };
    f.source.set_statuses([
        env("/a.yaml", SourceState::Found, 1),
        env("/b.yaml", SourceState::Found, 2),
        // A user-listed file is not part of the default entry, even when it is also found.
        status("/work/x.yaml", SourceState::Found, 9, None),
    ]);
    let rows = f.run(f.service.rows()).unwrap();
    assert_eq!(rows[0].contexts, 3);
    assert_eq!(rows[0].state, Some(SourceState::Found));
}

#[test]
fn a_missing_default_file_is_reported_as_missing() {
    let f = Fixture::with_defaults();
    f.source.set_statuses([SourceStatus {
        message: Some("File not found".into()),
        state: SourceState::Missing,
        contexts: 0,
        ..default_status(0)
    }]);
    let rows = f.run(f.service.rows()).unwrap();
    assert_eq!(rows[0].state, Some(SourceState::Missing));
    assert!(rows[0].is_error());
}

#[test]
fn a_file_repeating_the_default_path_shows_the_status_of_that_path() {
    let f = Fixture::new([UserSource::default_source(), file("/home/me/.kube/config")]);
    f.source.set_statuses([default_status(3)]);
    let rows = f.run(f.service.rows()).unwrap();
    assert_eq!(rows[0].contexts, 3);
    assert_eq!(rows[1].state, Some(SourceState::Found));
    assert_eq!(rows[1].contexts, 0, "the first entry owns the contexts");
    assert_eq!(
        rows[1].message.as_deref(),
        Some("Same path as an earlier source")
    );
}

#[test]
fn a_repeated_broken_file_shows_its_error_on_both_rows() {
    let f = Fixture::new([UserSource::default_source(), file("/home/me/.kube/config")]);
    f.source.set_statuses([SourceStatus {
        state: SourceState::Invalid,
        message: Some("Not a valid kubeconfig".into()),
        ..default_status(0)
    }]);
    let rows = f.run(f.service.rows()).unwrap();
    assert!(rows[0].is_error());
    assert!(rows[1].is_error());
    assert_eq!(rows[1].message.as_deref(), Some("Not a valid kubeconfig"));
}

#[test]
fn a_source_not_read_yet_has_no_state() {
    let f = Fixture::new([file("/work/new.yaml")]);
    f.source.set_statuses([]);
    let rows = f.run(f.service.rows()).unwrap();
    assert_eq!(rows[0].state, None);
    assert!(!rows[0].is_error());
}

#[test]
fn rows_mark_the_files_oxikube_stored() {
    let path = stored("prod.yaml");
    let f = Fixture::new([UserSource::file(&path), file("/work/a.yaml")]);
    let rows = f.run(f.service.rows()).unwrap();
    assert!(rows[0].stored);
    assert!(!rows[1].stored);
    assert_eq!(rows[1].label, "/work/a.yaml");
}

#[test]
fn reload_rereads_the_sources_and_picks_up_what_changed() {
    let f = Fixture::with_defaults();
    let diff = SourcesChanged {
        added: vec![context("fresh", "/work/a.yaml")],
        ..SourcesChanged::default()
    };
    f.source.script().reload.push_ok(diff.clone());
    let changed = f.run(f.service.reload()).unwrap();
    assert_eq!(changed, diff);
    assert!(
        f.source
            .recorded_calls()
            .contains(&ClusterSourceCall::Reload)
    );

    // And the rows read afterwards see the new status.
    f.source.set_statuses([default_status(1)]);
    assert_eq!(f.run(f.service.rows()).unwrap()[0].contexts, 1);
}
