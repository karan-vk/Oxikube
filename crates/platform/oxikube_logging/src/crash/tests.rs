//! Unit tests of the crash report: content, redaction, file handling.

use super::*;

// Fake secrets. Nothing here is a real credential.
const BEARER: &str = "s3cr3t-bearer-token-0123456789";
const JWT: &str = "eyJhbGciOiJSUzI1NiIsImtpZCI6ImZha2UifQ.eyJzdWIiOiJzeXN0ZW06c2VydmljZWFjY291bnQ6ZGVmYXVsdDpmYWtlIn0.c2lnbmF0dXJlLWZha2U";

fn report(message: &str) -> PanicReport {
    PanicReport {
        message: message.to_owned(),
        location: Some("src/main.rs:10:5".to_owned()),
        thread: Some("main".to_owned()),
        backtrace: "   0: oxikube::main\n   1: std::rt::lang_start".to_owned(),
    }
}

fn config(dir: &Path) -> CrashConfig {
    CrashConfig::new(dir, "9.9.9-test")
}

#[test]
fn the_report_names_the_panic_and_the_build() {
    let dir = tempfile::tempdir().unwrap();
    let text = render_report(&config(dir.path()), &report("index out of bounds"));
    for expected in [
        "Oxikube crash report",
        "version: 9.9.9-test",
        "thread: main",
        "location: src/main.rs:10:5",
        "message: index out of bounds",
        "oxikube::main",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
}

#[test]
fn secrets_in_the_message_and_the_backtrace_are_masked() {
    let dir = tempfile::tempdir().unwrap();
    let mut panic = report(&format!(
        "request failed: Authorization: Bearer {BEARER} (token {JWT})"
    ));
    panic.backtrace = format!("   0: call(headers = Authorization: Bearer {BEARER})");
    let path = write_report(&config(dir.path()), &panic).unwrap();
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains("request failed"), "{text}");
    for secret in [BEARER, JWT] {
        assert!(!text.contains(secret), "{secret} leaked:\n{text}");
    }
}

#[test]
fn reports_in_the_same_second_do_not_overwrite_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let first = write_report(&config, &report("one")).unwrap();
    let second = write_report(&config, &report("two")).unwrap();
    assert_ne!(first, second);
    assert!(fs::read_to_string(first).unwrap().contains("message: one"));
    assert!(fs::read_to_string(second).unwrap().contains("message: two"));
}

#[test]
fn only_the_newest_reports_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    // Older reports sort first by name.
    for stamp in ["20200101T000000Z", "20200102T000000Z", "20200103T000000Z"] {
        fs::write(
            dir.path().join(format!("crash-{stamp}-1.log")),
            "old report",
        )
        .unwrap();
    }
    fs::write(dir.path().join("notes.txt"), "not a report").unwrap();
    let mut config = config(dir.path());
    config.max_reports = 2;
    let newest = write_report(&config, &report("new")).unwrap();

    let mut names: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), 3, "2 reports + notes.txt: {names:?}");
    assert!(names.contains(&"notes.txt".to_owned()));
    assert!(names.contains(&newest.file_name().unwrap().to_string_lossy().into_owned()));
    assert!(!names.iter().any(|n| n.contains("20200101")));
}

#[cfg(unix)]
#[test]
fn reports_are_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let path = write_report(&config(dir.path()), &report("x")).unwrap();
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn an_unwritable_directory_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    // The "directory" is a file.
    let blocker = dir.path().join("file");
    fs::write(&blocker, "x").unwrap();
    assert!(write_report(&config(&blocker.join("crashes")), &report("x")).is_err());
}
