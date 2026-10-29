//! The commands run in the container, the meter, the stderr tail.

use std::time::{Duration, Instant};

use oxikube_domain::ErrorKind;
use parking_lot::Mutex;
use std::sync::Arc;

use super::super::failure::Tail;
use super::super::progress::ProgressMeter;
use super::super::target::{STOP_ON_EOF, cat_command, tar_create_command, tar_extract_command};
use super::super::{PROGRESS_INTERVAL, TransferProgress};

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_owned()).collect()
}

/// The tar arguments after the `STOP_ON_EOF` wrapper.
fn tar(path: &str) -> Vec<String> {
    let command = tar_create_command(path).expect(path);
    assert_eq!(command[..2], ["sh", "-c"]);
    assert_eq!(
        command[2], STOP_ON_EOF,
        "the script is fixed; paths never enter it"
    );
    command[4..].to_vec()
}

#[test]
fn download_archives_from_the_parent_with_a_relative_name() {
    assert_eq!(
        tar("/var/log"),
        argv(&["tar", "-cf", "-", "-C", "/var", "log"])
    );
    assert_eq!(
        tar("/var/log/"),
        argv(&["tar", "-cf", "-", "-C", "/var", "log"])
    );
    assert_eq!(tar("/etc"), argv(&["tar", "-cf", "-", "-C", "/", "etc"]));
    assert_eq!(tar("/"), argv(&["tar", "-cf", "-", "-C", "/", "."]));
    assert_eq!(
        tar("data/a.txt"),
        argv(&["tar", "-cf", "-", "-C", "data", "a.txt"])
    );
    assert_eq!(tar("a.txt"), argv(&["tar", "-cf", "-", "-C", ".", "a.txt"]));
    assert_eq!(
        tar("/tmp/./x//y"),
        argv(&["tar", "-cf", "-", "-C", "/tmp/x", "y"])
    );
    // A name that looks like an option is made a path; shell syntax stays an argument.
    assert_eq!(
        tar("/tmp/-rf"),
        argv(&["tar", "-cf", "-", "-C", "/tmp", "./-rf"])
    );
    assert_eq!(
        tar("/tmp/$(id);x"),
        argv(&["tar", "-cf", "-", "-C", "/tmp", "$(id);x"])
    );
}

#[test]
fn bad_paths_are_refused_before_any_exec() {
    for path in ["", "  ", "/tmp/../etc", "..", "a\0b"] {
        let error = tar_create_command(path).expect_err(path);
        assert_eq!(error.kind(), ErrorKind::Validation, "{path:?}");
        assert_eq!(
            tar_extract_command(path).expect_err(path).kind(),
            ErrorKind::Validation
        );
    }
    assert_eq!(
        cat_command("/tmp/").expect_err("dir").kind(),
        ErrorKind::Validation
    );
    assert_eq!(
        cat_command("/").expect_err("root").kind(),
        ErrorKind::Validation
    );
}

#[test]
fn upload_and_cat_commands() {
    assert_eq!(
        tar_extract_command("/srv/data").expect("dir"),
        argv(&["tar", "-xmf", "-", "-C", "/srv/data"])
    );
    assert_eq!(
        cat_command("/etc/hosts").expect("file"),
        argv(&["cat", "/etc/hosts"])
    );
    assert_eq!(cat_command("-n").expect("file"), argv(&["cat", "./-n"]));
}

#[test]
fn progress_is_reported_at_most_ten_times_a_second_plus_the_end() {
    let seen = Arc::new(Mutex::new(Vec::<TransferProgress>::new()));
    let sink = seen.clone();
    let mut meter = ProgressMeter::new(
        Some(Arc::new(move |progress| sink.lock().push(progress))),
        Some(1_000_000),
    );
    let start = Instant::now();
    // 1 000 chunks of 1 000 bytes over one second.
    for i in 0..1_000u64 {
        meter.advance_at(1_000, start + Duration::from_millis(i));
    }
    meter.finish();
    meter.finish();
    let seen = seen.lock();
    assert!(seen.len() <= 11, "{} reports in one second", seen.len());
    assert!(seen.len() >= 10, "{} reports", seen.len());
    for pair in seen.windows(2) {
        assert!(pair[0].bytes <= pair[1].bytes, "progress never goes back");
    }
    let last = seen.last().expect("a final report");
    assert_eq!(last.bytes, 1_000_000);
    assert_eq!(last.total, Some(1_000_000));
    assert_eq!(last.fraction(), Some(1.0));
    assert_eq!(meter.bytes(), 1_000_000);
    assert!(PROGRESS_INTERVAL >= Duration::from_millis(100));
}

#[test]
fn progress_fraction_needs_a_total() {
    let half = TransferProgress {
        bytes: 5,
        total: Some(10),
    };
    assert_eq!(half.fraction(), Some(0.5));
    assert_eq!(
        TransferProgress {
            bytes: 5,
            total: None
        }
        .fraction(),
        None
    );
    assert_eq!(
        TransferProgress {
            bytes: 5,
            total: Some(0)
        }
        .fraction(),
        None
    );
    // A tar stream is a little larger than its files: clamped.
    assert_eq!(
        TransferProgress {
            bytes: 12,
            total: Some(10)
        }
        .fraction(),
        Some(1.0)
    );
}

#[test]
fn the_stderr_tail_is_bounded_and_skips_the_leading_slash_notice() {
    let mut tail = Tail::default();
    tail.push(b"tar: Removing leading `/' from member names\n");
    tail.push(&vec![b'x'; 10_000]);
    tail.push(b"\ntar: a: Cannot open: Permission denied\n");
    let text = tail.text();
    assert!(
        text.ends_with("tar: a: Cannot open: Permission denied"),
        "{text}"
    );
    assert!(text.len() <= 2048, "{}", text.len());
    assert!(!text.contains("Removing leading"));
}
