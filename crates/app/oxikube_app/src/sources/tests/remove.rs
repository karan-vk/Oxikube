//! Removing a source: delete only what Oxikube stored.

use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::command::KubeconfigSourceRef;
use oxikube_testkit::FsCall;

use super::*;

fn remove_file(path: &str) -> KubeconfigSourceRef {
    KubeconfigSourceRef::File { path: path.into() }
}

#[test]
fn removing_a_stored_kubeconfig_deletes_the_file_and_the_entry() {
    let path = stored("prod.yaml");
    let f = Fixture::new([UserSource::default_source(), UserSource::file(&path)]);
    f.fs.insert(path.clone(), kubeconfig(1).into_bytes());

    let change = f
        .run(f.service.remove(&remove_file(&path.display().to_string())))
        .unwrap();
    assert_eq!(change.deleted_file.as_deref(), Some(path.as_path()));
    assert!(f.fs.file(&path).is_none());
    assert_eq!(f.list.snapshot(), vec![UserSource::default_source()]);
    assert_eq!(f.source.user_sources(), f.list.snapshot());
}

#[test]
fn removing_a_user_owned_file_only_removes_the_entry() {
    let f = Fixture::new([file("/work/prod.yaml"), dir("/work/configs")]);
    f.fs.insert("/work/prod.yaml", kubeconfig(1).into_bytes());

    let change = f
        .run(f.service.remove(&remove_file("/work/prod.yaml")))
        .unwrap();
    assert_eq!(change.deleted_file, None);
    assert!(
        f.fs.file("/work/prod.yaml").is_some(),
        "the user's file is untouched"
    );
    assert!(
        !f.fs
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, FsCall::Remove(_))),
        "no delete call at all"
    );
    assert_eq!(f.list.snapshot(), vec![dir("/work/configs")]);
}

#[test]
fn removing_a_folder_or_the_default_entry_never_deletes_a_file() {
    let f = Fixture::new([UserSource::default_source(), dir(DIR)]);
    f.run(
        f.service
            .remove(&KubeconfigSourceRef::Dir { path: DIR.into() }),
    )
    .unwrap();
    f.run(f.service.remove(&KubeconfigSourceRef::Default))
        .unwrap();
    assert!(f.list.snapshot().is_empty());
    assert!(f.fs.recorded_calls().is_empty());
}

#[test]
fn a_path_that_climbs_out_of_the_directory_is_not_ours_to_delete() {
    let sneaky = format!("{DIR}/../secrets.yaml");
    let nested = format!("{DIR}/sub/inner.yaml");
    let f = Fixture::new([file(&sneaky), file(&nested)]);
    f.fs.insert("/config/secrets.yaml", b"keep me".to_vec());
    for path in [&sneaky, &nested] {
        assert!(!f.service.deletes_file_on_remove(&file(path)), "{path}");
        let change = f.run(f.service.remove(&remove_file(path))).unwrap();
        assert_eq!(change.deleted_file, None);
    }
    assert_eq!(f.fs.file("/config/secrets.yaml"), Some(b"keep me".to_vec()));
}

#[test]
fn the_confirm_text_can_tell_the_two_kinds_apart() {
    let f = Fixture::with_defaults();
    assert!(
        f.service
            .deletes_file_on_remove(&UserSource::file(stored("a.yaml")))
    );
    assert!(!f.service.deletes_file_on_remove(&file("/work/a.yaml")));
    assert!(!f.service.deletes_file_on_remove(&dir(DIR)));
    assert!(
        !f.service
            .deletes_file_on_remove(&UserSource::default_source())
    );
}

#[test]
fn a_source_that_is_not_listed_is_not_found() {
    let f = Fixture::with_defaults();
    let error = f
        .run(f.service.remove(&remove_file("/work/x.yaml")))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[test]
fn a_file_that_cannot_be_deleted_keeps_its_entry_so_the_removal_can_be_retried() {
    let path = stored("prod.yaml");
    let f = Fixture::new([UserSource::file(&path)]);
    f.fs.insert(path.clone(), kubeconfig(1).into_bytes());
    f.fs.script()
        .remove
        .push_err(OxiError::forbidden("no permission to remove the file"));
    let error = f
        .run(f.service.remove(&remove_file(&path.display().to_string())))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Forbidden);
    assert_eq!(f.list.snapshot(), vec![UserSource::file(&path)]);

    // The retry works.
    f.run(f.service.remove(&remove_file(&path.display().to_string())))
        .unwrap();
    assert!(f.list.snapshot().is_empty());
}

#[test]
fn a_stored_file_that_is_already_gone_is_not_an_error() {
    let path = stored("gone.yaml");
    let f = Fixture::new([UserSource::file(&path)]);
    f.run(f.service.remove(&remove_file(&path.display().to_string())))
        .unwrap();
    assert!(f.list.snapshot().is_empty());
}
