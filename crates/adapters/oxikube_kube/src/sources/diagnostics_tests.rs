//! `source_diagnostics` and `subscribe_diagnostics` through `Arc<dyn ClusterSourcePort>`
//! (E03-F439). No watcher, no sleeps: every change is followed by a call that reloads.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use futures::{FutureExt as _, StreamExt as _};
use oxikube_ports::{
    ClusterSourcePort, DiagnosticSeverity, SecretStorePort, SourceDiagnostic, UserSource,
};
use oxikube_testkit::FakeSecretStorePort;
use tempfile::TempDir;

use super::{KubeconfigSources, PastedDescriptor, SourcesConfig, WatchStatus};
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

/// A port, held as a trait object the way the UI holds it, over a default kubeconfig with
/// `default_names` and the given extra user sources and pasted descriptors. No watcher.
fn port(
    dir: &Path,
    default_names: &[&str],
    extra: &[UserSource],
    pasted: Vec<PastedDescriptor>,
) -> (Arc<KubeconfigSources>, Arc<dyn ClusterSourcePort>) {
    let kube = dir.join("home").join(".kube");
    fs::create_dir_all(&kube).unwrap();
    fs::write(kube.join("config"), yaml(default_names)).unwrap();
    let mut config = SourcesConfig::new(Env {
        platform: Platform::host(),
        home: Some(dir.join("home")),
        ..Env::default()
    });
    config.watch = false;
    config.extra_paths = extra.iter().filter_map(|s| s.path.clone()).collect();
    config.pasted = pasted;
    let secrets: Arc<dyn SecretStorePort> = Arc::new(FakeSecretStorePort::new());
    let adapter = Arc::new(KubeconfigSources::new(config, secrets).unwrap());
    (adapter.clone(), adapter)
}

fn about<'a>(list: &'a [SourceDiagnostic], path: &Path) -> Vec<&'a SourceDiagnostic> {
    list.iter()
        .filter(|d| d.path.as_deref() == Some(path))
        .collect()
}

#[tokio::test]
async fn an_unparsable_file_is_reported_by_path_without_its_content() {
    let dir = TempDir::new().unwrap();
    let bad = dir.path().join("broken.yaml");
    fs::write(&bad, format!("token: {TOKEN}\n{{{{ not yaml")).unwrap();
    let gone = dir.path().join("absent.yaml");
    let (_, port) = port(
        dir.path(),
        &["d1"],
        &[UserSource::file(&bad), UserSource::file(&gone)],
        Vec::new(),
    );

    let list = port.source_diagnostics().await.unwrap();

    let [broken] = about(&list, &bad)[..] else {
        panic!("one diagnostic for the broken file: {list:?}");
    };
    assert_eq!(broken.severity, DiagnosticSeverity::Warning);
    assert!(broken.message.contains("broken.yaml"), "{}", broken.message);
    let [missing] = about(&list, &gone)[..] else {
        panic!("one diagnostic for the missing file: {list:?}");
    };
    assert_eq!(missing.severity, DiagnosticSeverity::Info);
    assert!(
        !format!("{list:?}").contains(TOKEN),
        "diagnostics leaked file contents: {list:?}"
    );
}

#[tokio::test]
async fn a_shadowed_context_is_reported_against_the_file_that_loses() {
    let dir = TempDir::new().unwrap();
    let second = dir.path().join("second.yaml");
    fs::write(&second, yaml(&["shared", "only-second"])).unwrap();
    let (_, port) = port(
        dir.path(),
        &["shared"],
        &[UserSource::file(&second)],
        Vec::new(),
    );

    let list = port.source_diagnostics().await.unwrap();

    let [shadowed] = about(&list, &second)[..] else {
        panic!("one diagnostic for the shadowing: {list:?}");
    };
    assert_eq!(shadowed.severity, DiagnosticSeverity::Warning);
    assert!(shadowed.message.contains("shared"), "{}", shadowed.message);
}

#[tokio::test]
async fn a_pasted_kubeconfig_problem_has_no_path_and_names_the_paste() {
    let dir = TempDir::new().unwrap();
    let (_, port) = port(
        dir.path(),
        &["d1"],
        &[],
        vec![PastedDescriptor {
            id: "0123456789abcdef".into(),
            label: "gone".into(),
        }],
    );

    let list = port.source_diagnostics().await.unwrap();

    let pasted: Vec<_> = list
        .iter()
        .filter(|d| d.message.contains("pasted:0123456789abcdef"))
        .collect();
    assert_eq!(pasted.len(), 1, "{list:?}");
    assert_eq!(pasted[0].path, None, "a paste is not a file");
}

#[tokio::test]
async fn a_healthy_load_has_no_diagnostics_and_the_first_read_loads_the_sources() {
    let dir = TempDir::new().unwrap();
    let (_, port) = port(dir.path(), &["d1"], &[], Vec::new());
    // No other call first: asking for diagnostics reads the sources.
    assert_eq!(port.source_diagnostics().await.unwrap(), []);
}

#[tokio::test]
async fn the_port_list_follows_the_inherent_loader_diagnostics() {
    let dir = TempDir::new().unwrap();
    let bad = dir.path().join("broken.yaml");
    fs::write(&bad, "{{{ not yaml").unwrap();
    let (adapter, _) = port(dir.path(), &["d1"], &[UserSource::file(&bad)], Vec::new());

    let via_port = adapter.source_diagnostics().await.unwrap();
    let via_loader: Vec<_> = adapter
        .diagnostics()
        .iter()
        .map(|d| (d.severity(), d.to_string()))
        .collect();
    let mapped: Vec<_> = via_port
        .iter()
        .map(|d| {
            let severity = match d.severity {
                DiagnosticSeverity::Info => crate::kubeconfig::Severity::Info,
                DiagnosticSeverity::Warning => crate::kubeconfig::Severity::Warning,
            };
            (severity, d.message.clone())
        })
        .collect();
    assert_eq!(mapped, via_loader);
}

#[tokio::test]
async fn subscribers_hear_a_diagnostic_that_changes_no_context() {
    let dir = TempDir::new().unwrap();
    let extra = dir.path().join("extra.yaml");
    let (adapter, _) = port(dir.path(), &["d1"], &[UserSource::file(&extra)], Vec::new());
    let mut catalog = adapter.subscribe();
    let mut diagnostics = adapter.subscribe_diagnostics();
    // The first read only records what is there: nothing is announced.
    let first = adapter.source_diagnostics().await.unwrap();
    assert!(diagnostics.next().now_or_never().is_none());
    assert!(first.iter().any(|d| d.path.as_deref() == Some(&extra)));

    // A file that is not a kubeconfig appears: no context is added or removed, yet the list
    // changed, which is the case the catalog stream cannot report.
    fs::write(&extra, "{{{ not yaml").unwrap();
    let diff = adapter.reload().await.unwrap();
    assert!(diff.is_empty(), "{diff:?}");
    assert!(catalog.next().now_or_never().is_none());
    let heard = diagnostics.next().now_or_never().flatten().expect("heard");
    assert_eq!(heard, adapter.source_diagnostics().await.unwrap());
    assert_ne!(heard, first);

    // Reading again with nothing changed says nothing.
    adapter.reload().await.unwrap();
    assert!(diagnostics.next().now_or_never().is_none());

    // Repairing the file empties the finding.
    fs::write(&extra, yaml(&["x1"])).unwrap();
    adapter.reload().await.unwrap();
    let healed = diagnostics.next().now_or_never().flatten().expect("heard");
    assert!(healed.iter().all(|d| d.path.as_deref() != Some(&extra)));
}

#[tokio::test]
async fn a_directory_the_watcher_could_not_register_is_reported_and_announced() {
    let dir = TempDir::new().unwrap();
    let (adapter, _) = port(dir.path(), &["d1"], &[], Vec::new());
    adapter.contexts().await.unwrap();
    let mut diagnostics = adapter.subscribe_diagnostics();
    let unwatched = dir.path().join("kubeconfigs");

    adapter
        .inner
        .set_watch_status(WatchStatus::Active, vec![unwatched.clone()]);

    let heard = diagnostics.next().now_or_never().flatten().expect("heard");
    assert_eq!(adapter.source_diagnostics().await.unwrap(), heard);
    let [found] = about(&heard, &unwatched)[..] else {
        panic!("one diagnostic for the directory: {heard:?}");
    };
    assert_eq!(found.severity, DiagnosticSeverity::Warning);
    assert!(found.message.contains("watched"), "{}", found.message);

    // The watcher registering the directory later clears it.
    adapter.inner.set_watch_status(WatchStatus::Active, vec![]);
    let cleared = diagnostics.next().now_or_never().flatten().expect("heard");
    assert!(about(&cleared, &unwatched).is_empty());
}

#[tokio::test]
async fn a_watcher_outcome_before_the_first_load_does_not_announce_a_partial_list() {
    let dir = TempDir::new().unwrap();
    let bad = dir.path().join("broken.yaml");
    fs::write(&bad, "{{{ not yaml").unwrap();
    let (adapter, _) = port(dir.path(), &["d1"], &[UserSource::file(&bad)], Vec::new());
    let mut diagnostics = adapter.subscribe_diagnostics();
    let unwatched = dir.path().join("kubeconfigs");

    // Nothing is loaded yet, so the unwatched directory alone is not the full list.
    adapter
        .inner
        .set_watch_status(WatchStatus::Active, vec![unwatched.clone()]);
    assert!(diagnostics.next().now_or_never().is_none());

    // The first read reports everything, the unwatched directory and the broken file.
    let first = adapter.source_diagnostics().await.unwrap();
    assert_eq!(about(&first, &unwatched).len(), 1, "{first:?}");
    assert_eq!(about(&first, &bad).len(), 1, "{first:?}");
    assert!(diagnostics.next().now_or_never().is_none());
}
