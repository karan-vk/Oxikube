//! The run-time source list (`set_user_sources`), the per-source statuses and pasted-text
//! validation (E06-S05). No watcher, no sleeps: every change is followed by a call that reloads.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_ports::cluster_source::{SourceState, SourceStatus};
use oxikube_ports::{ClusterSourcePort, UserSource};
use oxikube_testkit::FakeSecretStorePort;
use tempfile::TempDir;

use super::{KubeconfigSources, SourcesConfig};
use crate::kubeconfig::{Env, Platform};

const TOKEN: &str = "s3cr3t-token-do-not-leak";

fn yaml(names: &[&str]) -> String {
    let mut out = String::from("apiVersion: v1\nkind: Config\nclusters:\n");
    for name in names {
        out.push_str(&format!(
            "- name: {name}\n  cluster:\n    server: https://{name}\n"
        ));
    }
    out.push_str("users:\n");
    for name in names {
        out.push_str(&format!("- name: {name}\n  user:\n    token: {TOKEN}\n"));
    }
    out.push_str("contexts:\n");
    for name in names {
        out.push_str(&format!(
            "- name: {name}\n  context:\n    cluster: {name}\n    user: {name}\n"
        ));
    }
    out
}

/// An adapter whose default path is `<dir>/home/.kube/config` (written with `default_names`),
/// without a watcher.
fn adapter(dir: &Path, default_names: &[&str]) -> KubeconfigSources {
    let kube = dir.join("home").join(".kube");
    fs::create_dir_all(&kube).unwrap();
    if !default_names.is_empty() {
        fs::write(kube.join("config"), yaml(default_names)).unwrap();
    }
    let mut config = SourcesConfig::new(Env {
        platform: Platform::host(),
        home: Some(dir.join("home")),
        ..Env::default()
    });
    config.watch = false;
    KubeconfigSources::new(config, Arc::new(FakeSecretStorePort::new())).unwrap()
}

fn context_names(contexts: &[oxikube_ports::ClusterContext]) -> Vec<String> {
    contexts.iter().map(|c| c.context.to_string()).collect()
}

#[tokio::test]
async fn user_sources_replace_the_list_and_reload() {
    let dir = TempDir::new().unwrap();
    let extra = dir.path().join("extra.yaml");
    fs::write(&extra, yaml(&["x1", "x2"])).unwrap();
    let sources = adapter(dir.path(), &["d1"]);
    assert_eq!(
        context_names(&sources.contexts().await.unwrap()),
        ["d1"],
        "the default tier is read until the list says otherwise"
    );

    let diff = sources
        .set_user_sources(&[UserSource::default_source(), UserSource::file(&extra)])
        .await
        .unwrap();
    assert_eq!(context_names(&diff.added), ["x1", "x2"]);
    assert_eq!(
        context_names(&sources.contexts().await.unwrap()),
        ["d1", "x1", "x2"]
    );

    // Dropping the default entry stops reading ~/.kube/config.
    let diff = sources
        .set_user_sources(&[UserSource::file(&extra)])
        .await
        .unwrap();
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(
        context_names(&sources.contexts().await.unwrap()),
        ["x1", "x2"]
    );
    let listed = sources.sources().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path.as_deref(), Some(extra.as_path()));
}

#[tokio::test]
async fn an_empty_list_reads_nothing() {
    let dir = TempDir::new().unwrap();
    let sources = adapter(dir.path(), &["d1"]);
    sources.set_user_sources(&[]).await.unwrap();
    assert!(sources.contexts().await.unwrap().is_empty());
    assert!(sources.source_statuses().await.unwrap().is_empty());
}

#[tokio::test]
async fn subscribers_hear_about_a_list_change() {
    use futures::{FutureExt as _, StreamExt as _};
    let dir = TempDir::new().unwrap();
    let extra = dir.path().join("extra.yaml");
    fs::write(&extra, yaml(&["x1"])).unwrap();
    let sources = adapter(dir.path(), &[]);
    sources.contexts().await.unwrap();
    let mut changes = sources.subscribe();
    sources
        .set_user_sources(&[UserSource::default_source(), UserSource::file(&extra)])
        .await
        .unwrap();
    let diff = changes.next().now_or_never().flatten().expect("a diff");
    assert_eq!(context_names(&diff.added), ["x1"]);
}

fn status_of<'a>(statuses: &'a [SourceStatus], path: &Path) -> &'a SourceStatus {
    statuses
        .iter()
        .find(|s| s.source.path.as_deref() == Some(path))
        .unwrap_or_else(|| panic!("no status for {}", path.display()))
}

#[tokio::test]
async fn statuses_say_found_missing_blank_and_invalid() {
    let dir = TempDir::new().unwrap();
    let good = dir.path().join("good.yaml");
    let broken = dir.path().join("broken.yaml");
    let blank = dir.path().join("blank.yaml");
    let missing = dir.path().join("missing.yaml");
    fs::write(&good, yaml(&["g1", "g2"])).unwrap();
    // A token that must never come back in a message.
    fs::write(&broken, format!("contexts: [\n  token: {TOKEN}\n")).unwrap();
    fs::write(&blank, "").unwrap();
    let sources = adapter(dir.path(), &["d1"]);
    sources
        .set_user_sources(&[
            UserSource::default_source(),
            UserSource::file(&good),
            UserSource::file(&broken),
            UserSource::file(&blank),
            UserSource::file(&missing),
        ])
        .await
        .unwrap();
    let statuses = sources.source_statuses().await.unwrap();
    assert_eq!(statuses.len(), 5);

    assert_eq!(statuses[0].state, SourceState::Found);
    assert_eq!(statuses[0].contexts, 1);
    let found = status_of(&statuses, &good);
    assert_eq!((found.state, found.contexts), (SourceState::Found, 2));
    assert_eq!(found.message, None);
    let invalid = status_of(&statuses, &broken);
    assert_eq!(invalid.state, SourceState::Invalid);
    assert_eq!(invalid.contexts, 0);
    assert_eq!(invalid.message.as_deref(), Some("Not a valid kubeconfig"));
    assert_eq!(status_of(&statuses, &blank).state, SourceState::Blank);
    assert_eq!(status_of(&statuses, &missing).state, SourceState::Missing);

    // The broken file does not stop the others from loading.
    assert_eq!(
        context_names(&sources.contexts().await.unwrap()),
        ["d1", "g1", "g2"]
    );
    for status in &statuses {
        assert!(
            !format!("{status:?}").contains(TOKEN),
            "a status must not carry file content"
        );
    }
}

#[tokio::test]
async fn a_directory_reports_its_files_and_loads_the_rest() {
    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("configs");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("a.yaml"), yaml(&["a1"])).unwrap();
    fs::write(folder.join("b.yaml"), "this: [is not\n").unwrap();
    let empty = dir.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let nowhere = dir.path().join("nowhere");
    let sources = adapter(dir.path(), &[]);
    sources
        .set_user_sources(&[
            UserSource::dir(&folder),
            UserSource::dir(&empty),
            UserSource::dir(&nowhere),
        ])
        .await
        .unwrap();
    let statuses = sources.source_statuses().await.unwrap();

    let partly = status_of(&statuses, &folder);
    assert_eq!((partly.state, partly.contexts), (SourceState::Found, 1));
    let message = partly.message.as_deref().unwrap();
    assert!(
        message.starts_with("1 of 2 files skipped: b.yaml"),
        "{message}"
    );
    assert_eq!(status_of(&statuses, &empty).state, SourceState::Blank);
    assert_eq!(status_of(&statuses, &nowhere).state, SourceState::Missing);
    assert_eq!(context_names(&sources.contexts().await.unwrap()), ["a1"]);
}

#[tokio::test]
async fn validation_counts_contexts_and_never_echoes_the_text() {
    let dir = TempDir::new().unwrap();
    let sources = adapter(dir.path(), &[]);
    assert_eq!(
        sources
            .validate_kubeconfig(&yaml(&["a", "b", "c"]))
            .await
            .unwrap(),
        3
    );
    let secret = format!("clusters: [\n token: {TOKEN}");
    for bad in ["", "   \n", "just words", secret.as_str()] {
        let error = sources.validate_kubeconfig(bad).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation, "{bad:?}");
        assert!(!error.message().contains(TOKEN));
    }
    // Validating stores nothing.
    assert!(sources.contexts().await.unwrap().is_empty());
}

/// A watcher-enabled adapter whose default path is under `<dir>/home`, plus a directory
/// `<dir>/added` to add later.
fn watched_adapter(dir: &Path) -> (KubeconfigSources, std::path::PathBuf) {
    let watched = dir.join("added");
    fs::create_dir(&watched).unwrap();
    fs::create_dir_all(dir.join("home").join(".kube")).unwrap();
    let mut config = SourcesConfig::new(Env {
        platform: Platform::host(),
        home: Some(dir.join("home")),
        ..Env::default()
    });
    config.debounce = std::time::Duration::from_millis(50);
    let sources = KubeconfigSources::new(config, Arc::new(FakeSecretStorePort::new())).unwrap();
    (sources, watched)
}

/// Drops a kubeconfig into `watched` until a `SourcesChanged` arrives (polling wait with a
/// deadline, no fixed sleep; the 60 s safety poll is far outside the window).
async fn drop_file_until_seen(
    root: &Path,
    watched: &Path,
    events: &mut futures::stream::BoxStream<'static, oxikube_ports::SourcesChanged>,
) -> oxikube_ports::SourcesChanged {
    use std::time::{Duration, Instant};

    use futures::StreamExt as _;

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let staged = root.join(format!("staged-{attempt}"));
        fs::write(&staged, yaml(&["dropped"])).unwrap();
        fs::rename(&staged, watched.join("new.yaml")).unwrap();
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        if let Ok(Some(diff)) = tokio::time::timeout(wait, events.next()).await {
            return diff;
        }
        assert!(Instant::now() < deadline, "no SourcesChanged within 3 s");
    }
}

/// The watcher follows the list: a directory added at run time is watched, so a file dropped
/// into it is picked up without a manual reload. Real `notify` watcher.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_watcher_follows_a_directory_added_at_run_time() {
    let dir = TempDir::new().unwrap();
    let (sources, watched) = watched_adapter(dir.path());
    sources.contexts().await.unwrap();
    sources.wait_for_watcher().await;
    sources
        .set_user_sources(&[UserSource::default_source(), UserSource::dir(&watched)])
        .await
        .unwrap();
    let mut events = sources.subscribe();

    let diff = drop_file_until_seen(dir.path(), &watched, &mut events).await;
    assert_eq!(context_names(&diff.added), ["dropped"]);
}

/// A list change that lands while the watcher task is still starting must not be lost. On a
/// current-thread runtime the task cannot run before the first await, so the list is changed
/// (and `rewatch` bumped, exactly as `set_user_sources` does) before the first registration.
#[tokio::test]
async fn a_list_change_made_before_the_watcher_starts_is_still_watched() {
    let dir = TempDir::new().unwrap();
    let (sources, watched) = watched_adapter(dir.path());
    sources.inner.config.write().extra_paths = vec![watched.clone()];
    sources
        .inner
        .rewatch
        .send_modify(|generation| *generation += 1);
    let mut events = sources.subscribe();

    assert_eq!(
        sources.wait_for_watcher().await,
        crate::sources::WatchStatus::Active
    );
    let diff = drop_file_until_seen(dir.path(), &watched, &mut events).await;
    assert_eq!(context_names(&diff.added), ["dropped"]);
}

/// The story's performance case: 20 contexts across three files. The files are parsed on the
/// blocking pool inside `set_user_sources` / `reload`, never on the caller's thread, and one read
/// is well inside a frame budget multiple.
#[tokio::test]
async fn twenty_contexts_across_three_files_load_in_one_read() {
    use std::time::Instant;

    let dir = TempDir::new().unwrap();
    let names: Vec<String> = (0..20).map(|i| format!("ctx-{i:02}")).collect();
    let mut paths = Vec::new();
    for (n, chunk) in [&names[..7], &names[7..14], &names[14..]]
        .iter()
        .enumerate()
    {
        let path = dir.path().join(format!("file-{n}.yaml"));
        let chunk: Vec<&str> = chunk.iter().map(String::as_str).collect();
        fs::write(&path, yaml(&chunk)).unwrap();
        paths.push(path);
    }
    let sources = adapter(dir.path(), &[]);
    let list: Vec<UserSource> = paths.iter().map(UserSource::file).collect();
    let started = Instant::now();
    sources.set_user_sources(&list).await.unwrap();
    let elapsed = started.elapsed();
    eprintln!("20 contexts across 3 files: {elapsed:?}");

    assert_eq!(sources.contexts().await.unwrap().len(), 20);
    let statuses = sources.source_statuses().await.unwrap();
    assert_eq!(
        statuses.iter().map(|s| s.contexts).collect::<Vec<_>>(),
        [7, 7, 6]
    );
    assert!(elapsed < std::time::Duration::from_secs(2), "{elapsed:?}");
}
