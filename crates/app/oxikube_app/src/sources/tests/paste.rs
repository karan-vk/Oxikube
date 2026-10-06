//! Pasting a kubeconfig: validate, store as `<dir>/<name>.yaml` owner-only, list as a file.

use oxikube_domain::ErrorKind;
use oxikube_domain::command::{NewKubeconfigSource, PastedText};
use oxikube_testkit::FsCall;

use super::*;

const SECRET: &str = "s3cr3t-token-do-not-leak";

fn paste(name: &str, text: &str) -> NewKubeconfigSource {
    NewKubeconfigSource::Pasted {
        name: name.into(),
        text: PastedText::new(text),
    }
}

#[test]
fn a_valid_paste_is_stored_privately_and_listed_as_a_file() {
    let f = Fixture::with_defaults();
    let text = kubeconfig(2);
    let change = f.run(f.service.add(&paste("prod", &text))).unwrap();

    let path = stored("prod.yaml");
    assert_eq!(change.created_file.as_deref(), Some(path.as_path()));
    assert_eq!(change.contexts_in_paste, 2);
    assert_eq!(f.fs.file(&path), Some(text.into_bytes()));
    assert!(f.fs.is_private(&path), "written owner-only");
    // The text is never written the plain way: that would leave it group-readable.
    assert!(
        !f.fs
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, FsCall::Write(..)))
    );
    assert_eq!(
        f.list.snapshot(),
        vec![UserSource::default_source(), UserSource::file(&path)]
    );
    assert_eq!(f.source.user_sources(), f.list.snapshot());
}

#[test]
fn the_name_may_carry_the_extension() {
    let f = Fixture::new([]);
    f.run(f.service.add(&paste("staging.YML", &kubeconfig(1))))
        .unwrap();
    assert!(f.fs.file(stored("staging.yaml")).is_some());
}

#[test]
fn invalid_text_is_rejected_before_anything_is_written() {
    let f = Fixture::with_defaults();
    for text in ["", "   \n", "just some words", &format!("token: {SECRET}")] {
        let error = f.run(f.service.add(&paste("prod", text))).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation, "{text:?}");
        assert!(
            !format!("{error} {error:?}").contains(SECRET),
            "the error must not quote the text"
        );
    }
    assert!(
        !f.fs.recorded_calls().iter().any(|c| matches!(
            c,
            FsCall::Write(..) | FsCall::WritePrivate(..) | FsCall::Remove(..)
        )),
        "{:?}",
        f.fs.recorded_calls()
    );
    assert_eq!(f.list.snapshot(), vec![UserSource::default_source()]);
}

#[test]
fn the_text_is_validated_through_the_cluster_source_port() {
    let f = Fixture::new([]);
    f.source
        .script()
        .validate_kubeconfig
        .push_err(oxikube_domain::OxiError::validation(
            "the pasted text is not a valid kubeconfig",
        ));
    let error = f
        .run(f.service.add(&paste("prod", &kubeconfig(1))))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(f.fs.file(stored("prod.yaml")).is_none());
}

#[test]
fn names_that_could_escape_the_directory_are_rejected() {
    let f = Fixture::new([]);
    for name in [
        "../evil",
        "..",
        "a/b",
        "a\\b",
        "/etc/passwd",
        "C:evil",
        ".hidden",
        "..yaml",
        "",
        "   ",
        ".yaml",
        "nul",
        "COM1.yaml",
        "name with space",
        "tab\tname",
        "nul\0byte",
        &"x".repeat(65),
    ] {
        let error = f
            .run(f.service.add(&paste(name, &kubeconfig(1))))
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation, "{name:?}");
    }
    assert!(
        f.fs.recorded_calls().is_empty(),
        "no file call for a rejected name: {:?}",
        f.fs.recorded_calls()
    );
    assert!(f.list.snapshot().is_empty());
}

#[test]
fn an_existing_stored_kubeconfig_is_never_overwritten() {
    let f = Fixture::new([]);
    let fs = f.fs.clone();
    fs.insert(stored("prod.yaml"), b"original".to_vec());
    for name in ["prod", "PROD", "prod.yaml"] {
        let error = f
            .run(f.service.add(&paste(name, &kubeconfig(1))))
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Conflict, "{name}");
    }
    assert_eq!(f.fs.file(stored("prod.yaml")), Some(b"original".to_vec()));
    assert!(f.list.snapshot().is_empty());
}

#[test]
fn a_file_written_for_a_paste_is_deleted_when_the_list_cannot_be_saved() {
    let fs = Arc::new(FakeFsPort::new());
    let service = KubeconfigSourcesService::new(
        Arc::new(FakeClusterSourcePort::new()),
        fs.clone(),
        Arc::new(ReadOnlyList(vec![])),
        PathBuf::from(DIR),
    );
    let error = block_on(service.add(&paste("prod", &kubeconfig(1)))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert!(
        fs.file(stored("prod.yaml")).is_none(),
        "no orphan left behind"
    );
}

#[test]
fn a_paste_is_also_listed_when_the_directory_does_not_exist_yet() {
    // `list` of a missing directory is NotFound; the write then creates it.
    let f = Fixture::new([]);
    f.run(f.service.add(&paste("first", &kubeconfig(1))))
        .unwrap();
    assert!(f.fs.file(stored("first.yaml")).is_some());
}
