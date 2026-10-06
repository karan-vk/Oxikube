//! Adding a file, a folder and the default entry.

use oxikube_domain::ErrorKind;
use oxikube_domain::command::NewKubeconfigSource;
use oxikube_testkit::{ClusterSourceCall, FsCall};

use super::*;

fn add_file(path: &str) -> NewKubeconfigSource {
    NewKubeconfigSource::File { path: path.into() }
}

#[test]
fn adding_a_file_stores_it_and_tells_the_cluster_source() {
    let f = Fixture::with_defaults();
    let change = f.run(f.service.add(&add_file("/work/prod.yaml"))).unwrap();
    assert_eq!(change.source, file("/work/prod.yaml"));
    assert!(!change.unchanged);
    assert_eq!(change.created_file, None);

    let expected = vec![UserSource::default_source(), file("/work/prod.yaml")];
    assert_eq!(f.list.snapshot(), expected);
    assert_eq!(f.source.user_sources(), expected);
    // A file the user owns is read in place: nothing is written.
    assert!(
        f.fs.recorded_calls().is_empty(),
        "{:?}",
        f.fs.recorded_calls()
    );
}

#[test]
fn adding_a_folder_stores_it_as_a_folder() {
    let f = Fixture::with_defaults();
    f.run(f.service.add(&NewKubeconfigSource::Dir {
        path: "/work/configs".into(),
    }))
    .unwrap();
    assert_eq!(
        f.list.snapshot(),
        vec![UserSource::default_source(), dir("/work/configs")]
    );
    assert_eq!(f.source.user_sources(), f.list.snapshot());
}

#[test]
fn a_missing_or_broken_file_is_still_added_so_its_row_can_show_the_error() {
    // The service never reads the file: the cluster source reports what is wrong with it.
    let f = Fixture::new([]);
    f.run(f.service.add(&add_file("/work/does-not-exist.yaml")))
        .unwrap();
    assert_eq!(f.list.snapshot(), vec![file("/work/does-not-exist.yaml")]);
}

#[test]
fn adding_the_same_source_twice_changes_nothing_but_still_reloads() {
    let f = Fixture::with_defaults();
    f.run(f.service.add(&add_file("/work/a.yaml"))).unwrap();
    let again = f.run(f.service.add(&add_file("/work/a.yaml"))).unwrap();
    assert!(again.unchanged);
    assert_eq!(f.list.snapshot().len(), 2);
    let sets = f
        .source
        .recorded_calls()
        .into_iter()
        .filter(|c| matches!(c, ClusterSourceCall::SetUserSources(_)))
        .count();
    assert_eq!(sets, 2, "a repeated add re-reads the sources");
}

#[test]
fn the_default_entry_can_be_added_back() {
    let f = Fixture::new([file("/work/a.yaml")]);
    f.run(f.service.add(&NewKubeconfigSource::Default)).unwrap();
    assert_eq!(
        f.list.snapshot(),
        vec![file("/work/a.yaml"), UserSource::default_source()]
    );
}

#[test]
fn empty_and_relative_paths_are_rejected_without_touching_anything() {
    let f = Fixture::with_defaults();
    for path in ["", "   ", "relative/config", "~/config", "./config"] {
        let error = f.run(f.service.add(&add_file(path))).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation, "{path:?}");
    }
    assert_eq!(f.list.snapshot(), vec![UserSource::default_source()]);
    assert!(f.source.recorded_calls().is_empty());
    assert!(
        !f.fs
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, FsCall::Write(..) | FsCall::WritePrivate(..)))
    );
}

#[test]
fn a_list_that_cannot_be_saved_is_an_error_and_the_source_is_not_told() {
    let source = Arc::new(FakeClusterSourcePort::new());
    let service = KubeconfigSourcesService::new(
        source.clone(),
        Arc::new(FakeFsPort::new()),
        Arc::new(ReadOnlyList(vec![])),
        PathBuf::from(DIR),
    );
    let error = block_on(service.add(&add_file("/work/a.yaml"))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert!(source.recorded_calls().is_empty());
}

#[test]
fn apply_stored_pushes_the_stored_list_to_the_cluster_source() {
    // The user edited settings.json: the service re-applies what is stored.
    let f = Fixture::new([UserSource::default_source(), dir("/work/configs")]);
    f.run(f.service.apply_stored()).unwrap();
    assert_eq!(
        f.source.user_sources(),
        vec![UserSource::default_source(), dir("/work/configs")]
    );
}
