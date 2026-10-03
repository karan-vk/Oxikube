//! Deterministic tests: no watcher, no sleeps; every change is followed by a manual `reload()`.
//! The one watcher test at the end uses a real temp dir and a polling wait with a deadline.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::{FutureExt, StreamExt};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::secrets::SecretString;
use oxikube_ports::{SourceId, SourceKind};
use oxikube_testkit::FakeSecretStorePort;
use tempfile::TempDir;

use super::*;
use crate::kubeconfig::{Diagnostic, SourceStatus};

const TOKEN: &str = "s3cr3t-token-do-not-leak";

/// A kubeconfig with one cluster, user and context per `(name, server)`.
fn yaml(contexts: &[(&str, &str)], current: Option<&str>, token: &str, ns: Option<&str>) -> String {
    let mut out = String::from("apiVersion: v1\nkind: Config\n");
    if let Some(current) = current {
        out.push_str(&format!("current-context: {current}\n"));
    }
    out.push_str("clusters:\n");
    for (name, server) in contexts {
        out.push_str(&format!(
            "- name: {name}\n  cluster:\n    server: {server}\n"
        ));
    }
    out.push_str("users:\n");
    for (name, _) in contexts {
        out.push_str(&format!("- name: {name}\n  user:\n    token: {token}\n"));
    }
    out.push_str("contexts:\n");
    for (name, _) in contexts {
        out.push_str(&format!(
            "- name: {name}\n  context:\n    cluster: {name}\n    user: {name}\n"
        ));
        if let Some(ns) = ns {
            out.push_str(&format!("    namespace: {ns}\n"));
        }
    }
    out
}

fn simple(contexts: &[(&str, &str)]) -> String {
    yaml(contexts, None, TOKEN, None)
}

fn write(path: &Path, text: &str) {
    fs::write(path, text).expect("write fixture");
}

fn config_for(default: &Path) -> SourcesConfig {
    let mut config = SourcesConfig::new(None, Some(default.to_path_buf()));
    config.watch = false;
    config
}

fn adapter(config: SourcesConfig) -> (KubeconfigSources, Arc<FakeSecretStorePort>) {
    let secrets = Arc::new(FakeSecretStorePort::new());
    let sources = KubeconfigSources::new(config, secrets.clone()).expect("no watcher to start");
    (sources, secrets)
}

fn names(contexts: &[ClusterContext]) -> Vec<String> {
    contexts.iter().map(|c| c.context.to_string()).collect()
}

fn ctx_names(diff: &SourcesChanged) -> (Vec<String>, Vec<String>) {
    (names(&diff.added), names(&diff.changed))
}

/// The next event already delivered to `stream`, without waiting.
fn try_next(stream: &mut BoxStream<'static, SourcesChanged>) -> Option<SourcesChanged> {
    stream.next().now_or_never().flatten()
}

#[tokio::test]
async fn default_path_is_one_source_with_its_contexts() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    write(&path, &simple(&[("a", "https://a"), ("b", "https://b")]));
    let (sources, _) = adapter(config_for(&path));

    let listed = sources.sources().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, SourceId("default".into()));
    assert_eq!(listed[0].kind, SourceKind::KubeconfigFile);
    assert_eq!(listed[0].path.as_deref(), Some(path.as_path()));

    let contexts = sources.contexts().await.unwrap();
    assert_eq!(names(&contexts), ["a", "b"]);
    assert_eq!(contexts[0].server.as_deref(), Some("https://a"));
    assert_eq!(contexts[0].source, SourceId("default".into()));
    let key = fs::canonicalize(&path).unwrap();
    assert_eq!(
        contexts[0].cluster,
        ClusterId::new(&key.to_string_lossy(), &ContextName::new("a"))
    );
    assert!(sources.diagnostics().is_empty());
}

#[tokio::test]
async fn kubeconfig_env_replaces_the_default_path() {
    let dir = TempDir::new().unwrap();
    let default = dir.path().join("default");
    let one = dir.path().join("one");
    let two = dir.path().join("two");
    write(&default, &simple(&[("from-default", "https://d")]));
    write(&one, &simple(&[("one", "https://1")]));
    write(&two, &simple(&[("two", "https://2")]));
    let env: OsString = std::env::join_paths([&one, &two]).unwrap();
    let mut config = config_for(&default);
    config.kubeconfig_env = Some(env);
    let (sources, _) = adapter(config);

    let listed = sources.sources().await.unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|s| s.kind == SourceKind::Environment));
    let contexts = sources.contexts().await.unwrap();
    assert_eq!(names(&contexts), ["one", "two"]);
    assert_eq!(contexts[1].source, listed[1].id);
}

#[tokio::test]
async fn an_empty_kubeconfig_env_falls_back_to_the_default_path() {
    let dir = TempDir::new().unwrap();
    let default = dir.path().join("default");
    write(&default, &simple(&[("d", "https://d")]));
    let mut config = config_for(&default);
    config.kubeconfig_env = Some(OsString::new());
    let (sources, _) = adapter(config);
    assert_eq!(names(&sources.contexts().await.unwrap()), ["d"]);
}

#[tokio::test]
async fn a_new_file_in_a_directory_source_is_reported_as_added() {
    let dir = TempDir::new().unwrap();
    let extra = dir.path().join("extra");
    fs::create_dir(&extra).unwrap();
    write(&extra.join("one.yaml"), &simple(&[("one", "https://1")]));
    let mut config = config_for(&dir.path().join("missing"));
    config.extra_paths = vec![extra.clone()];
    let (sources, _) = adapter(config);
    assert_eq!(names(&sources.contexts().await.unwrap()), ["one"]);
    let mut events = sources.subscribe();

    write(&extra.join("two.yaml"), &simple(&[("two", "https://2")]));
    let diff = sources.reload().await.unwrap();

    assert_eq!(ctx_names(&diff), (vec!["two".into()], vec![]));
    assert!(diff.removed.is_empty());
    assert_eq!(
        diff.added[0].source,
        SourceId(format!("dir:{}", extra.display()))
    );
    assert_eq!(
        try_next(&mut events),
        Some(diff),
        "subscribers get the returned diff"
    );
    assert!(try_next(&mut events).is_none());
}

#[tokio::test]
async fn editing_the_server_is_reported_as_changed() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    write(&path, &simple(&[("a", "https://a"), ("b", "https://b")]));
    let (sources, _) = adapter(config_for(&path));
    sources.contexts().await.unwrap();

    write(&path, &simple(&[("a", "https://a"), ("b", "https://b2")]));
    let diff = sources.reload().await.unwrap();

    assert_eq!(ctx_names(&diff), (vec![], vec!["b".into()]));
    assert_eq!(diff.changed[0].server.as_deref(), Some("https://b2"));
    assert!(diff.removed.is_empty());
}

#[tokio::test]
async fn removing_a_context_is_reported_by_cluster_id() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    write(&path, &simple(&[("a", "https://a"), ("b", "https://b")]));
    let (sources, _) = adapter(config_for(&path));
    let before = sources.contexts().await.unwrap();

    write(&path, &simple(&[("a", "https://a")]));
    let diff = sources.reload().await.unwrap();

    assert_eq!(diff.removed, vec![before[1].cluster.clone()]);
    assert!(diff.added.is_empty() && diff.changed.is_empty());
}

#[tokio::test]
async fn namespace_user_and_credential_changes_are_reported_as_changed() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    let (sources, _) = adapter(config_for(&path));
    write(&path, &yaml(&[("a", "https://a")], None, "token-1", None));
    sources.contexts().await.unwrap();

    write(
        &path,
        &yaml(&[("a", "https://a")], None, "token-1", Some("team-x")),
    );
    let diff = sources.reload().await.unwrap();
    assert_eq!(diff.changed[0].default_namespace.as_deref(), Some("team-x"));

    // Only the token changed: the port-visible fields are identical, the diff still says so.
    write(
        &path,
        &yaml(&[("a", "https://a")], None, "token-2", Some("team-x")),
    );
    let diff = sources.reload().await.unwrap();
    assert_eq!(names(&diff.changed), ["a"]);
    assert_eq!(diff.changed[0].server.as_deref(), Some("https://a"));

    let renamed = yaml(&[("a", "https://a")], None, "token-2", Some("team-x"))
        .replace("user: a", "user: other")
        .replace("- name: a\n  user:", "- name: other\n  user:");
    write(&path, &renamed);
    assert_eq!(names(&sources.reload().await.unwrap().changed), ["a"]);
}

#[tokio::test]
async fn touching_a_file_without_changing_it_emits_nothing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    let text = yaml(&[("a", "https://a")], Some("a"), TOKEN, None);
    write(&path, &text);
    let (sources, _) = adapter(config_for(&path));
    sources.contexts().await.unwrap();
    let mut events = sources.subscribe();

    // A rewrite with identical content gets a new mtime; so does an explicit touch.
    write(&path, &text);
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(std::time::SystemTime::now())
        .unwrap();
    let diff = sources.reload().await.unwrap();

    assert!(diff.is_empty());
    assert!(try_next(&mut events).is_none());
}

#[tokio::test]
async fn reordered_hash_maps_in_a_user_entry_are_not_a_change() {
    // kube parses `as-user-extra` and exec `env` into HashMaps, whose iteration order is
    // randomised per instance. The fingerprint must not depend on it.
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    let mut text = simple(&[("a", "https://a")]);
    text = text.replace("    token: s3cr3t-token-do-not-leak\n", "");
    text = text.replace(
        "  user:\n",
        "  user:\n    as-user-extra:\n      k1: [v1]\n      k2: [v2]\n      k3: [v3]\n      k4: [v4]\n      k5: [v5]\n      k6: [v6]\n    exec:\n      apiVersion: client.authentication.k8s.io/v1\n      command: get-token\n      env:\n      - {name: A, value: '1', other: x, more: y, extra: z, last: w}\n",
    );
    write(&path, &text);
    let (sources, _) = adapter(config_for(&path));
    sources.contexts().await.unwrap();
    assert!(
        sources.diagnostics().is_empty(),
        "{:?}",
        sources.diagnostics()
    );

    for _ in 0..25 {
        write(&path, &text);
        assert!(sources.reload().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn an_external_current_context_change_reports_both_contexts() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    let two = [("a", "https://a"), ("b", "https://b"), ("c", "https://c")];
    write(&path, &yaml(&two, Some("a"), TOKEN, None));
    let (sources, _) = adapter(config_for(&path));
    sources.contexts().await.unwrap();
    assert_eq!(sources.current_context(), Some(ContextName::new("a")));

    // `kubectl config use-context b` rewrites only the current-context line.
    write(&path, &yaml(&two, Some("b"), TOKEN, None));
    let diff = sources.reload().await.unwrap();

    assert_eq!(names(&diff.changed), ["a", "b"], "c is untouched");
    assert!(diff.added.is_empty() && diff.removed.is_empty());
    assert_eq!(sources.current_context(), Some(ContextName::new("b")));
}

#[tokio::test]
async fn an_atomic_replace_by_rename_is_seen_by_reload() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    write(&path, &simple(&[("a", "https://a")]));
    let (sources, _) = adapter(config_for(&path));
    sources.contexts().await.unwrap();

    let staged = dir.path().join("config.new");
    write(&staged, &simple(&[("a", "https://a"), ("b", "https://b")]));
    fs::rename(&staged, &path).unwrap();
    let diff = sources.reload().await.unwrap();

    assert_eq!(ctx_names(&diff), (vec!["b".into()], vec![]));
}

#[tokio::test]
async fn a_broken_file_is_a_diagnostic_and_the_rest_still_load() {
    let dir = TempDir::new().unwrap();
    let good = dir.path().join("good");
    let bad = dir.path().join("bad");
    write(&good, &simple(&[("a", "https://a")]));
    write(&bad, &format!("clusters: [unclosed {TOKEN}\n"));
    let env = std::env::join_paths([&good, &bad, &dir.path().join("absent")]).unwrap();
    let mut config = config_for(&good);
    config.kubeconfig_env = Some(env);
    let (sources, _) = adapter(config);

    assert_eq!(names(&sources.contexts().await.unwrap()), ["a"]);
    let diagnostics = sources.diagnostics();
    assert!(diagnostics.contains(&Diagnostic::Unparsable { path: bad.clone() }));
    assert!(diagnostics.contains(&Diagnostic::MissingFile {
        path: dir.path().join("absent")
    }));
    let shown = format!("{diagnostics:?}");
    assert!(
        !shown.contains(TOKEN),
        "diagnostics leaked file contents: {shown}"
    );
}

#[tokio::test]
async fn diagnostics_follow_the_files_across_reloads() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    write(&path, &simple(&[("a", "https://a")]));
    let (sources, _) = adapter(config_for(&path));
    sources.contexts().await.unwrap();
    assert!(sources.diagnostics().is_empty());

    write(&path, "{{{ not yaml");
    let diff = sources.reload().await.unwrap();
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(
        sources.diagnostics(),
        [Diagnostic::Unparsable { path: path.clone() }]
    );

    write(&path, &simple(&[("a", "https://a")]));
    let diff = sources.reload().await.unwrap();
    assert_eq!(names(&diff.added), ["a"]);
    assert!(sources.diagnostics().is_empty());
}

#[tokio::test]
async fn directory_sources_skip_hidden_backup_and_nested_files() {
    let dir = TempDir::new().unwrap();
    let extra = dir.path().join("extra");
    fs::create_dir_all(extra.join("nested")).unwrap();
    write(&extra.join("b.yaml"), &simple(&[("b", "https://b")]));
    write(&extra.join("a"), &simple(&[("a", "https://a")]));
    for ignored in [".hidden", "a~", "b.yaml.bak", ".a.swp", "config.lock"] {
        write(&extra.join(ignored), &simple(&[("ignored", "https://x")]));
    }
    write(&extra.join("nested/c"), &simple(&[("nested", "https://n")]));
    let mut config = config_for(&dir.path().join("missing"));
    config.extra_paths = vec![extra];
    let (sources, _) = adapter(config);

    assert_eq!(
        names(&sources.contexts().await.unwrap()),
        ["a", "b"],
        "sorted by file name"
    );
    assert!(
        sources
            .diagnostics()
            .iter()
            .all(|d| !matches!(d, Diagnostic::Unparsable { .. }))
    );
}

#[tokio::test]
async fn duplicate_context_names_across_sources_resolve_first_wins() {
    let dir = TempDir::new().unwrap();
    let default = dir.path().join("config");
    let extra = dir.path().join("extra.yaml");
    write(&default, &simple(&[("shared", "https://default")]));
    write(
        &extra,
        &simple(&[("shared", "https://extra"), ("only-extra", "https://e")]),
    );
    let mut config = config_for(&default);
    config.extra_paths = vec![extra.clone(), default.clone()];
    let (sources, _) = adapter(config);

    let contexts = sources.contexts().await.unwrap();
    assert_eq!(names(&contexts), ["shared", "only-extra"]);
    assert_eq!(contexts[0].server.as_deref(), Some("https://default"));
    assert_eq!(contexts[0].source, SourceId("default".into()));
    assert_eq!(
        contexts[1].source,
        SourceId(format!("file:{}", extra.display()))
    );
    assert!(sources.diagnostics().iter().any(|d| matches!(
        d,
        Diagnostic::DuplicateContext { context, .. } if context.as_str() == "shared"
    )));
}

#[tokio::test]
async fn the_merged_kubeconfig_is_exposed_for_the_pool() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    write(&path, &simple(&[("a", "https://a")]));
    let (sources, _) = adapter(config_for(&path));
    assert!(sources.kubeconfig().is_none(), "nothing loaded yet");
    sources.reload().await.unwrap();
    let merged = sources.kubeconfig().unwrap();
    assert_eq!(merged.contexts.len(), 1);
    let shown = format!("{sources:?}");
    assert!(!shown.contains(TOKEN));
}

#[tokio::test]
async fn a_first_reload_reports_everything_as_added() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    write(&path, &simple(&[("a", "https://a")]));
    let (sources, _) = adapter(config_for(&path));
    let mut events = sources.subscribe();
    let diff = sources.reload().await.unwrap();
    assert_eq!(names(&diff.added), ["a"]);
    assert_eq!(try_next(&mut events), Some(diff));
}

#[tokio::test]
async fn kubeconfig_env_and_an_added_directory_load_together_in_order() {
    let dir = TempDir::new().unwrap();
    let env_file = dir.path().join("env-config");
    let extra = dir.path().join("extra");
    fs::create_dir(&extra).unwrap();
    write(&env_file, &simple(&[("from-env", "https://e")]));
    write(&extra.join("k.yaml"), &simple(&[("from-dir", "https://d")]));
    let mut config = config_for(&dir.path().join("unused-default"));
    config.kubeconfig_env = Some(env_file.clone().into_os_string());
    config.extra_paths = vec![extra.clone()];
    let (sources, _) = adapter(config);

    let listed = sources.sources().await.unwrap();
    assert_eq!(
        listed.iter().map(|s| s.kind).collect::<Vec<_>>(),
        [SourceKind::Environment, SourceKind::KubeconfigDir]
    );
    let contexts = sources.contexts().await.unwrap();
    assert_eq!(names(&contexts), ["from-env", "from-dir"]);
    assert_eq!(contexts[1].source, listed[1].id);
}

// --- pasted kubeconfigs -------------------------------------------------------------------

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else {
            found.push(path);
        }
    }
    found.sort();
    found
}

#[tokio::test]
async fn a_pasted_kubeconfig_lives_in_the_secret_store_and_never_in_a_file() {
    let dir = TempDir::new().unwrap();
    let default = dir.path().join("config");
    write(&default, &simple(&[("from-file", "https://f")]));
    let files_before = files_under(dir.path());
    let (sources, secrets) = adapter(config_for(&default));
    sources.contexts().await.unwrap();
    let mut events = sources.subscribe();

    let text = simple(&[("pasted", "https://p")]);
    let descriptor = sources
        .add_pasted("  work cluster ", SecretString::from(text.clone()))
        .await
        .unwrap();

    // Stored under the keychain key, and only there.
    let key = descriptor.secret_key().unwrap();
    assert_eq!(key.namespace(), "kubeconfig-paste");
    use oxikube_ports::secrets::ExposeSecret;
    assert_eq!(secrets.peek(&key).unwrap().expose_secret(), text);
    assert_eq!(files_under(dir.path()), files_before, "no file was written");
    assert_eq!(descriptor.label, "work cluster");
    let shown = format!("{descriptor:?} {sources:?} {:?}", sources.pasted());
    assert!(
        !shown.contains(TOKEN),
        "descriptor/Debug leaked the text: {shown}"
    );

    // It is in the catalog, attributed to its own source, and subscribers heard about it.
    let contexts = sources.contexts().await.unwrap();
    assert_eq!(names(&contexts), ["from-file", "pasted"]);
    assert_eq!(
        contexts[1].source,
        SourceId(format!("pasted:{}", descriptor.id))
    );
    let listed = sources.sources().await.unwrap();
    assert_eq!(listed.last().unwrap().label, "Pasted: work cluster");
    assert_eq!(listed.last().unwrap().path, None);
    assert_eq!(names(&try_next(&mut events).unwrap().added), ["pasted"]);

    // Removing it deletes the secret and the contexts.
    assert!(sources.remove_pasted(&descriptor.id).await.unwrap());
    assert!(secrets.keys().is_empty());
    assert_eq!(names(&sources.contexts().await.unwrap()), ["from-file"]);
    assert_eq!(try_next(&mut events).unwrap().removed.len(), 1);
    assert!(!sources.remove_pasted(&descriptor.id).await.unwrap());
}

#[tokio::test]
async fn pasted_kubeconfigs_are_restored_from_descriptors_after_a_restart() {
    let dir = TempDir::new().unwrap();
    let default = dir.path().join("missing");
    let (first, secrets) = adapter(config_for(&default));
    let descriptor = first
        .add_pasted("p", SecretString::from(simple(&[("pasted", "https://p")])))
        .await
        .unwrap();

    // A new adapter given the persisted descriptors and the same keychain.
    let mut config = config_for(&default);
    config.pasted = vec![descriptor.clone()];
    let second = KubeconfigSources::new(config, secrets.clone()).unwrap();
    assert_eq!(names(&second.contexts().await.unwrap()), ["pasted"]);
    assert_eq!(second.pasted(), std::slice::from_ref(&descriptor));

    // The text is cached after the first read: a later keychain read is not needed.
    let before = secrets.recorded_calls().len();
    second.reload().await.unwrap();
    second.reload().await.unwrap();
    assert_eq!(
        secrets.recorded_calls().len(),
        before,
        "reloads do not hit the keychain again"
    );
}

#[tokio::test]
async fn a_pasted_kubeconfig_missing_from_the_keychain_is_a_diagnostic_not_an_error() {
    let dir = TempDir::new().unwrap();
    let (sources, _) = adapter({
        let mut config = config_for(&dir.path().join("missing"));
        config.pasted = vec![PastedDescriptor {
            id: "0123456789abcdef".into(),
            label: "gone".into(),
        }];
        config
    });
    assert!(sources.contexts().await.unwrap().is_empty());
    assert!(
        sources
            .diagnostics()
            .iter()
            .any(|d| matches!(d, Diagnostic::MissingFile { .. }))
    );
    let listed = sources.sources().await.unwrap();
    assert!(listed.iter().any(|s| s.id.0 == "pasted:0123456789abcdef"));
    let status = sources
        .inner
        .current
        .read()
        .as_ref()
        .unwrap()
        .loaded
        .sources
        .last()
        .unwrap()
        .status;
    assert_eq!(status, SourceStatus::Missing);
}

#[tokio::test]
async fn pasting_the_same_text_twice_keeps_one_entry_with_the_new_label() {
    let dir = TempDir::new().unwrap();
    let (sources, secrets) = adapter(config_for(&dir.path().join("missing")));
    let text = simple(&[("p", "https://p")]);
    let first = sources
        .add_pasted("one", SecretString::from(text.clone()))
        .await
        .unwrap();
    let second = sources
        .add_pasted("two", SecretString::from(text))
        .await
        .unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(sources.pasted().len(), 1);
    assert_eq!(sources.pasted()[0].label, "two");
    assert_eq!(secrets.keys().len(), 1);
}

#[tokio::test]
async fn pasting_text_that_is_not_a_kubeconfig_is_refused_without_storing_it() {
    let dir = TempDir::new().unwrap();
    let (sources, secrets) = adapter(config_for(&dir.path().join("missing")));
    for text in [format!("clusters: [unclosed {TOKEN}"), "   \n".to_owned()] {
        let err = sources
            .add_pasted("x", SecretString::from(text))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Validation);
        assert!(!err.message().contains(TOKEN), "{}", err.message());
    }
    assert!(secrets.keys().is_empty());
    assert!(sources.pasted().is_empty());
}

#[tokio::test]
async fn a_file_context_shadows_a_pasted_one_with_the_same_name() {
    let dir = TempDir::new().unwrap();
    let default = dir.path().join("config");
    write(&default, &simple(&[("shared", "https://file")]));
    let (sources, _) = adapter(config_for(&default));
    sources
        .add_pasted(
            "p",
            SecretString::from(simple(&[("shared", "https://pasted")])),
        )
        .await
        .unwrap();
    let contexts = sources.contexts().await.unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].server.as_deref(), Some("https://file"));
    assert!(
        sources
            .diagnostics()
            .iter()
            .any(|d| matches!(d, Diagnostic::DuplicateContext { .. }))
    );
}

// --- watcher ------------------------------------------------------------------------------

fn watching_config(default: &Path) -> SourcesConfig {
    let mut config = config_for(default);
    config.watch = true;
    config.debounce = Duration::from_millis(50);
    config
}

/// Replace `target` with new content by atomic rename every 250 ms until `events` yields a
/// diff, or fail after two seconds. Repeating the replacement (with different content each
/// time) tolerates a late first FSEvents delivery; the assertion is that a diff arrives.
async fn replace_until_event(
    dir: &Path,
    target: &Path,
    events: &mut BoxStream<'static, SourcesChanged>,
) -> SourcesChanged {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let staged = dir.join(format!("config.new.{attempt}"));
        write(
            &staged,
            &simple(&[("a", "https://a"), ("b", &format!("https://b{attempt}"))]),
        );
        fs::rename(&staged, target).unwrap();
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_millis(250));
        if let Ok(Some(diff)) = tokio::time::timeout(wait, events.next()).await {
            return diff;
        }
        assert!(Instant::now() < deadline, "no SourcesChanged within 2 s");
    }
}

fn mentions_b(diff: &SourcesChanged) -> bool {
    diff.added
        .iter()
        .chain(&diff.changed)
        .any(|c| c.context.as_str() == "b")
}

/// Real `notify` watcher on a temp dir, a kubeconfig replaced by atomic rename, and a polling
/// wait with a two-second deadline (no fixed sleep). Not a GPUI test: the crate has no GPUI.
///
/// The watcher registers asynchronously, so the test first waits for `wait_for_watcher`. The
/// 60 s safety poll is far outside the window, so only the watcher can satisfy it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_watcher_reports_an_atomic_replace_within_two_seconds() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    write(&path, &simple(&[("a", "https://a")]));
    let (sources, _) = adapter(watching_config(&path));
    sources.contexts().await.unwrap();
    let mut events = sources.subscribe();
    assert_eq!(sources.wait_for_watcher().await, WatchStatus::Active);

    let diff = replace_until_event(dir.path(), &path, &mut events).await;
    assert!(mentions_b(&diff), "{diff:?}");
}

/// `~/.kube/config -> elsewhere` (dotfile managers, Nix): writes land in the target's
/// directory, which must be watched too.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_watcher_sees_writes_through_a_symlinked_kubeconfig() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real");
    let links = dir.path().join("links");
    fs::create_dir(&real).unwrap();
    fs::create_dir(&links).unwrap();
    let target = real.join("config");
    write(&target, &simple(&[("a", "https://a")]));
    let link = links.join("config");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let (sources, _) = adapter(watching_config(&link));
    assert_eq!(names(&sources.contexts().await.unwrap()), ["a"]);
    let mut events = sources.subscribe();
    assert_eq!(sources.wait_for_watcher().await, WatchStatus::Active);

    let diff = replace_until_event(&real, &target, &mut events).await;
    assert!(mentions_b(&diff), "{diff:?}");
}

/// A burst of writes inside the debounce window becomes one reload that sees the final state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_burst_of_writes_is_coalesced_into_one_reload() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config");
    write(&path, &simple(&[("a", "https://a")]));
    let mut config = watching_config(&path);
    config.debounce = Duration::from_millis(300);
    let (sources, _) = adapter(config);
    sources.contexts().await.unwrap();
    let mut events = sources.subscribe();
    assert_eq!(sources.wait_for_watcher().await, WatchStatus::Active);

    for i in 1..=5 {
        write(&path, &simple(&[("a", &format!("https://a{i}"))]));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let first = tokio::time::timeout(Duration::from_secs(3), events.next())
        .await
        .expect("an event within the deadline")
        .unwrap();
    assert_eq!(
        first.changed[0].server.as_deref(),
        Some("https://a5"),
        "{first:?}"
    );
    // Nothing further: the intermediate states were never loaded on their own.
    assert!(
        tokio::time::timeout(Duration::from_millis(600), events.next())
            .await
            .is_err()
    );
}

/// With nothing to watch (the directory does not exist yet) the watcher reports `Failed` and
/// the safety poll alone picks up the file once it appears.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn when_the_watcher_cannot_start_the_safety_poll_still_finds_changes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("not-yet").join("config");
    let mut config = watching_config(&path);
    config.poll_interval = Duration::from_millis(100);
    let (sources, _) = adapter(config);
    assert!(sources.contexts().await.unwrap().is_empty());
    let mut events = sources.subscribe();
    assert!(matches!(
        sources.wait_for_watcher().await,
        WatchStatus::Failed(_)
    ));

    fs::create_dir(path.parent().unwrap()).unwrap();
    write(&path, &simple(&[("late", "https://l")]));
    let diff = tokio::time::timeout(Duration::from_secs(2), events.next())
        .await
        .expect("the poll reloads within the deadline")
        .unwrap();
    assert_eq!(names(&diff.added), ["late"]);
}

#[tokio::test]
async fn watching_without_a_runtime_is_an_error_and_disabled_starts_nothing() {
    let dir = TempDir::new().unwrap();
    let mut config = config_for(&dir.path().join("config"));
    let (sources, _) = adapter(config.clone());
    assert_eq!(sources.watch_status(), WatchStatus::Disabled);
    assert_eq!(sources.wait_for_watcher().await, WatchStatus::Disabled);
    config.watch = true;
    let handle = std::thread::spawn(move || {
        KubeconfigSources::new(config, Arc::new(FakeSecretStorePort::new())).map(|_| ())
    });
    let err = handle.join().unwrap().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Internal);
}
