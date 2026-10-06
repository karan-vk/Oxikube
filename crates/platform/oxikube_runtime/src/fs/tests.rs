use std::time::{Duration, Instant};

use futures::StreamExt as _;
use oxikube_domain::ErrorKind;
use oxikube_ports::{EntryKind, FsPort};
use tempfile::TempDir;

use super::StdFs;

#[tokio::test]
async fn write_then_read_round_trips_and_creates_parents() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a").join("b").join("file.txt");
    StdFs.write(&path, b"one").await.unwrap();
    assert_eq!(StdFs.read(&path).await.unwrap(), b"one");
    StdFs.write(&path, b"two").await.unwrap();
    assert_eq!(StdFs.read(&path).await.unwrap(), b"two");
    // No temp file is left behind.
    let names: Vec<_> = StdFs
        .list(path.parent().unwrap())
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["file.txt"]);
}

#[tokio::test]
async fn reading_a_missing_file_is_not_found() {
    let dir = TempDir::new().unwrap();
    let error = StdFs.read(&dir.path().join("nope")).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[cfg(unix)]
#[tokio::test]
async fn private_writes_are_owner_only_from_creation() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("kubeconfigs");
    let path = folder.join("prod.yaml");
    StdFs.write_private(&path, b"token: x").await.unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&folder), 0o700, "a directory it creates is owner-only");

    // Replacing the file keeps it private, whatever the umask.
    StdFs.write_private(&path, b"token: y").await.unwrap();
    assert_eq!(mode(&path), 0o600);
    assert_eq!(StdFs.read(&path).await.unwrap(), b"token: y");
}

#[tokio::test]
async fn remove_reports_whether_there_was_a_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("x");
    StdFs.write(&path, b"x").await.unwrap();
    assert!(StdFs.remove(&path).await.unwrap());
    assert!(!StdFs.remove(&path).await.unwrap());
    assert_eq!(
        StdFs.read(&path).await.unwrap_err().kind(),
        ErrorKind::NotFound
    );
}

#[tokio::test]
async fn list_orders_by_path_and_tells_files_from_directories() {
    let dir = TempDir::new().unwrap();
    StdFs.write(&dir.path().join("b.txt"), b"bb").await.unwrap();
    StdFs.write(&dir.path().join("a.txt"), b"a").await.unwrap();
    std::fs::create_dir(dir.path().join("c")).unwrap();
    let entries = StdFs.list(dir.path()).await.unwrap();
    let shape: Vec<_> = entries.iter().map(|e| (e.kind, e.size)).collect();
    assert_eq!(
        shape,
        [
            (EntryKind::File, Some(1)),
            (EntryKind::File, Some(2)),
            (EntryKind::Dir, None)
        ]
    );
    assert!(entries.windows(2).all(|w| w[0].path < w[1].path));
    let error = StdFs.list(&dir.path().join("missing")).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

/// A real watcher on a temp dir, polled with a deadline (no fixed sleep).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watch_reports_a_created_file_and_stops_when_dropped() {
    let dir = TempDir::new().unwrap();
    let mut events = StdFs.watch(dir.path());
    let target = dir.path().join("new.txt");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut attempt = 0;
    loop {
        attempt += 1;
        StdFs
            .write(&target, format!("v{attempt}").as_bytes())
            .await
            .unwrap();
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        if let Ok(Some(event)) = tokio::time::timeout(wait, events.next()).await
            && event.path.file_name() == target.file_name()
        {
            break;
        }
        assert!(Instant::now() < deadline, "no event within 3 s");
    }
    drop(events);
}
