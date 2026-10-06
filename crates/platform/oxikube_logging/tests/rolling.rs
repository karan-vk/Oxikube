//! The rolling file subscriber built by `oxikube_logging::build`: what lands in the file, what
//! never does, how many files are kept, and live filter changes.

use oxikube_logging::{DEFAULT_DIRECTIVES, LogConfig, LogError, SetOutcome, build};
use std::path::Path;

// Fake secrets. Nothing here is a real credential.
const BEARER: &str = "s3cr3t-bearer-token-0123456789";
const SECRET_DATA: &str = "cGFzc3dvcmQtZmFrZS12YWx1ZQ==";
const KEY_DATA: &str = "LS0tLS1CRUdJTiBSU0EgUFJJVkFURSBLRVktLS0tLWZha2VrZXlkYXRh";

fn config(dir: &Path) -> LogConfig {
    let mut config = LogConfig::new(dir);
    config.honour_rust_log = false;
    config
}

fn log_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    files
}

fn all_text(dir: &Path) -> String {
    log_files(dir)
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect()
}

#[test]
fn events_reach_a_dated_log_file_with_secrets_masked() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    let (dispatch, guard) = build(&config(&logs)).unwrap();
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!("cluster connected");
        tracing::info!(token = BEARER, "refreshing credentials");
        tracing::warn!("upstream said: Authorization: Bearer {BEARER}");
        tracing::info!(
            "applying manifest:\napiVersion: v1\nkind: Secret\ndata:\n  password: {SECRET_DATA}\n  key: {KEY_DATA}\n"
        );
    });
    drop(guard);

    let files = log_files(&logs);
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("oxikube.") && name.ends_with(".log"),
        "unexpected file name {name}"
    );
    let text = std::fs::read_to_string(&files[0]).unwrap();
    assert!(text.contains("cluster connected"), "{text}");
    assert!(text.contains("refreshing credentials"), "{text}");
    for secret in [BEARER, SECRET_DATA, KEY_DATA] {
        assert!(!text.contains(secret), "{secret} leaked:\n{text}");
    }
}

#[test]
fn old_files_are_pruned_to_the_configured_count() {
    let dir = tempfile::tempdir().unwrap();
    for day in 1..=6 {
        std::fs::write(
            dir.path().join(format!("oxikube.2020-01-0{day}.log")),
            "old",
        )
        .unwrap();
    }
    let mut config = config(dir.path());
    config.max_files = 3;
    let (dispatch, guard) = build(&config).unwrap();
    tracing::dispatcher::with_default(&dispatch, || tracing::info!("today"));
    drop(guard);

    let files = log_files(dir.path());
    assert!(files.len() <= 3, "{files:?}");
    // The newest old files survive, the oldest do not, and today's file exists.
    assert!(
        !files
            .iter()
            .any(|f| f.to_string_lossy().contains("2020-01-01"))
    );
    assert!(all_text(dir.path()).contains("today"));
}

#[test]
fn the_filter_can_be_changed_while_running() {
    let dir = tempfile::tempdir().unwrap();
    let (dispatch, guard) = build(&config(dir.path())).unwrap();
    let handle = guard.handle();
    assert_eq!(handle.directives(), DEFAULT_DIRECTIVES);
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::debug!("debug-off");
        assert_eq!(handle.set_directives("debug").unwrap(), SetOutcome::Applied);
        tracing::debug!("debug-on");
        // An invalid text is refused and the working filter stays.
        let err = handle.set_directives("info=[").unwrap_err();
        assert!(matches!(err, LogError::Directives { .. }), "{err}");
        tracing::debug!("debug-still-on");
    });
    drop(guard);
    let text = all_text(dir.path());
    assert!(!text.contains("debug-off"), "{text}");
    assert!(text.contains("debug-on"), "{text}");
    assert!(text.contains("debug-still-on"), "{text}");
}

#[test]
fn configured_directives_start_the_filter() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(dir.path());
    config.directives = Some("error".to_owned());
    let (dispatch, guard) = build(&config).unwrap();
    assert_eq!(guard.handle().directives(), "error");
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::warn!("warn-hidden");
        tracing::error!("error-shown");
    });
    drop(guard);
    let text = all_text(dir.path());
    assert!(
        !text.contains("warn-hidden") && text.contains("error-shown"),
        "{text}"
    );

    // Text that does not parse falls back to the shipped default.
    let dir = tempfile::tempdir().unwrap();
    let mut config = self::config(dir.path());
    config.directives = Some("info=[".to_owned());
    let (_dispatch, guard) = build(&config).unwrap();
    assert_eq!(guard.handle().directives(), DEFAULT_DIRECTIVES);
}

#[test]
fn an_unwritable_log_directory_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("file");
    std::fs::write(&blocker, "x").unwrap();
    let err = build(&config(&blocker.join("logs")))
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(err, LogError::File { .. }), "{err}");
}
