//! Deterministic tests: no watcher, no sleeps; every change is followed by a manual `reload()`.
//! The tests in the watcher section at the end start the real watcher task on temp dirs and
//! wait with deadlines instead of fixed sleeps.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::{FutureExt, StreamExt};
use oxikube_domain::ErrorKind;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::secrets::SecretString;
use oxikube_ports::{SourceId, SourceKind};

use crate::kubeconfig::in_cluster_cluster_id;
use crate::pool::{
    ClientPool, ContextDefinition, KubeClientFactory, PoolConfig, ProxyEnv, SystemClock,
};
use oxikube_testkit::FakeSecretStorePort;
use tempfile::TempDir;

use super::*;
use crate::kubeconfig::{Diagnostic, Env, InClusterSkip, Platform, SourceStatus};

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

/// `<dir>/home/.kube/config`, with its directory created: the default path of [`config_for`].
fn default_in(dir: &Path) -> PathBuf {
    let kube = dir.join("home").join(".kube");
    fs::create_dir_all(&kube).expect("create .kube");
    kube.join("config")
}

/// A config whose default path is `default`, which must be `<home>/.kube/config` (the home
/// directory need not exist). No `KUBECONFIG`, not in a cluster, no watcher.
fn config_for(default: &Path) -> SourcesConfig {
    assert!(default.ends_with(".kube/config"), "{}", default.display());
    let home = default.parent().and_then(Path::parent).unwrap();
    let mut config = SourcesConfig::new(Env {
        platform: Platform::host(),
        home: Some(home.to_path_buf()),
        ..Env::default()
    });
    config.watch = false;
    config
}

/// A config with no default path at all (no home directory).
fn config_without_default() -> SourcesConfig {
    let mut config = SourcesConfig::new(Env::default());
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
    let path = default_in(dir.path());
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
async fn contexts_carry_the_names_of_their_cluster_and_user() {
    let dir = TempDir::new().unwrap();
    let path = default_in(dir.path());
    write(
        &path,
        "apiVersion: v1\nkind: Config\n\
         clusters:\n- name: prod-eu\n  cluster:\n    server: https://eu\n\
         users:\n- name: alice\n  user:\n    token: s3cr3t-token-do-not-leak\n\
         contexts:\n- name: eu\n  context:\n    cluster: prod-eu\n    user: alice\n",
    );
    let (sources, _) = adapter(config_for(&path));
    let contexts = sources.contexts().await.unwrap();
    assert_eq!(contexts[0].cluster_name.as_deref(), Some("prod-eu"));
    assert_eq!(contexts[0].user.as_deref(), Some("alice"));
    assert_eq!(contexts[0].problem, None);
    // Names only: nothing of the credentials reaches the port.
    assert!(!format!("{contexts:?}").contains("s3cr3t"));
}

#[tokio::test]
async fn a_context_naming_a_missing_cluster_or_user_is_kept_and_flagged() {
    let dir = TempDir::new().unwrap();
    let path = default_in(dir.path());
    write(
        &path,
        "apiVersion: v1\nkind: Config\n\
         clusters:\n- name: ok\n  cluster:\n    server: https://ok\n\
         users:\n- name: bob\n  user:\n    token: t\n\
         contexts:\n\
         - name: good\n  context:\n    cluster: ok\n    user: bob\n\
         - name: no-cluster\n  context:\n    cluster: gone\n    user: bob\n\
         - name: no-user\n  context:\n    cluster: ok\n    user: nobody\n\
         - name: anonymous\n  context:\n    cluster: ok\n",
    );
    let (sources, _) = adapter(config_for(&path));
    let contexts = sources.contexts().await.unwrap();
    let problem = |name: &str| {
        contexts
            .iter()
            .find(|c| c.context.as_str() == name)
            .unwrap_or_else(|| panic!("{name} is listed"))
            .problem
            .clone()
    };
    assert_eq!(
        names(&contexts),
        ["good", "no-cluster", "no-user", "anonymous"]
    );
    assert_eq!(problem("good"), None);
    assert!(problem("no-cluster").is_some_and(|p| p.contains("\"gone\"")));
    assert!(problem("no-user").is_some_and(|p| p.contains("\"nobody\"")));
    // A context without a user is legitimate (anonymous access): not a problem.
    assert_eq!(problem("anonymous"), None);
}

#[tokio::test]
async fn kubeconfig_env_replaces_the_default_path() {
    let dir = TempDir::new().unwrap();
    let default = default_in(dir.path());
    let one = dir.path().join("one");
    let two = dir.path().join("two");
    write(&default, &simple(&[("from-default", "https://d")]));
    write(&one, &simple(&[("one", "https://1")]));
    write(&two, &simple(&[("two", "https://2")]));
    let env: OsString = std::env::join_paths([&one, &two]).unwrap();
    let mut config = config_for(&default);
    config.env.kubeconfig = Some(env);
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
    let default = default_in(dir.path());
    write(&default, &simple(&[("d", "https://d")]));
    let mut config = config_for(&default);
    config.env.kubeconfig = Some(OsString::new());
    let (sources, _) = adapter(config);
    assert_eq!(names(&sources.contexts().await.unwrap()), ["d"]);
}

#[tokio::test]
async fn a_new_file_in_a_directory_source_is_reported_as_added() {
    let dir = TempDir::new().unwrap();
    let extra = dir.path().join("extra");
    fs::create_dir(&extra).unwrap();
    write(&extra.join("one.yaml"), &simple(&[("one", "https://1")]));
    let mut config = config_without_default();
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
    let path = default_in(dir.path());
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
    let path = default_in(dir.path());
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
    let path = default_in(dir.path());
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
    let path = default_in(dir.path());
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
    let path = default_in(dir.path());
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
    let path = default_in(dir.path());
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
    let path = default_in(dir.path());
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
    let mut config = config_for(&default_in(dir.path()));
    config.env.kubeconfig = Some(env);
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
    let path = default_in(dir.path());
    write(&path, &simple(&[("a", "https://a")]));
    let (sources, _) = adapter(config_for(&path));
    sources.contexts().await.unwrap();
    assert!(sources.diagnostics().is_empty());

    write(&path, "{{{ not yaml");
    let diff = sources.reload().await.unwrap();
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(
        sources.diagnostics(),
        [
            Diagnostic::Unparsable { path: path.clone() },
            // The catalog is empty, but a broken kubeconfig never falls back to in-cluster.
            Diagnostic::InClusterSkipped {
                reason: InClusterSkip::BrokenKubeconfig
            },
        ]
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
    let mut config = config_without_default();
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
    let default = default_in(dir.path());
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
    let path = default_in(dir.path());
    write(&path, &simple(&[("a", "https://a")]));
    let (sources, _) = adapter(config_for(&path));
    assert!(sources.loaded().is_none(), "nothing loaded yet");
    sources.reload().await.unwrap();
    let loaded = sources.loaded().unwrap();
    assert_eq!(loaded.merged.contexts.len(), 1);
    let shown = format!("{sources:?}");
    assert!(!shown.contains(TOKEN));
}

#[tokio::test]
async fn a_first_reload_reports_everything_as_added() {
    let dir = TempDir::new().unwrap();
    let path = default_in(dir.path());
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
    let mut config = config_for(&default_in(dir.path()));
    config.env.kubeconfig = Some(env_file.clone().into_os_string());
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

/// kubectl parity: a set, non-empty `KUBECONFIG` that splits to no path still selects the
/// `KUBECONFIG` tier, so the default path is not read.
#[tokio::test]
async fn a_separators_only_kubeconfig_env_reads_no_file_not_the_default_path() {
    let dir = TempDir::new().unwrap();
    let default = default_in(dir.path());
    write(&default, &simple(&[("d", "https://d")]));
    for value in [":", "::", ";"] {
        let mut config = config_for(&default);
        config.env.kubeconfig = Some(OsString::from(value));
        let (sources, _) = adapter(config);

        assert!(sources.contexts().await.unwrap().is_empty(), "{value:?}");
        assert!(sources.sources().await.unwrap().is_empty(), "{value:?}");
        assert!(
            sources
                .diagnostics()
                .contains(&Diagnostic::InClusterSkipped {
                    reason: InClusterSkip::NotInCluster
                }),
            "{value:?}"
        );
    }
}

/// kubectl accepts `KUBECONFIG=/a:/a`, and settings may repeat a path: each source is listed
/// once, keeping the first, so `SourceId`s stay unique.
#[tokio::test]
async fn repeated_kubeconfig_entries_and_user_added_paths_list_each_source_once() {
    let dir = TempDir::new().unwrap();
    let one = dir.path().join("one");
    let two = dir.path().join("two");
    let extra = dir.path().join("extra");
    fs::create_dir(&extra).unwrap();
    write(&one, &simple(&[("one", "https://1")]));
    write(&two, &simple(&[("two", "https://2")]));
    write(&extra.join("k.yaml"), &simple(&[("from-dir", "https://d")]));
    let mut config = config_for(&default_in(dir.path()));
    config.env.kubeconfig = Some(std::env::join_paths([&one, &one, &two]).unwrap());
    config.extra_paths = vec![extra.clone(), one.clone(), extra.clone(), two.clone()];
    let (sources, _) = adapter(config);

    let listed = sources.sources().await.unwrap();
    let ids: Vec<SourceId> = listed.iter().map(|s| s.id.clone()).collect();
    assert_eq!(
        ids,
        [
            SourceId(format!("env:{}", one.display())),
            SourceId(format!("env:{}", two.display())),
            SourceId(format!("dir:{}", extra.display())),
        ]
    );
    let contexts = sources.contexts().await.unwrap();
    assert_eq!(names(&contexts), ["one", "two", "from-dir"]);
    assert!(contexts.iter().all(|c| ids.contains(&c.source)));
}

/// An `Env` that looks like a pod (service host, port and mounted service account).
fn pod_env(config: &mut SourcesConfig) {
    config.env.kubernetes_service_host = Some("10.0.0.1".into());
    config.env.kubernetes_service_port = Some("443".into());
    config.env.service_account_mounted = true;
    config.env.service_account_namespace = Some("apps".into());
}

#[tokio::test]
async fn in_a_pod_with_no_kubeconfig_the_service_account_is_an_in_cluster_source() {
    let dir = TempDir::new().unwrap();
    let default = default_in(dir.path());
    let mut config = config_for(&default);
    pod_env(&mut config);
    let (sources, _) = adapter(config);

    let contexts = sources.contexts().await.unwrap();
    assert_eq!(names(&contexts), ["in-cluster"]);
    assert_eq!(contexts[0].cluster, in_cluster_cluster_id());
    assert_eq!(contexts[0].server.as_deref(), Some("https://10.0.0.1"));
    assert_eq!(contexts[0].default_namespace.as_deref(), Some("apps"));
    let listed = sources.sources().await.unwrap();
    let in_cluster = listed.last().unwrap();
    assert_eq!(in_cluster.kind, SourceKind::InCluster);
    assert_eq!(in_cluster.path, None);
    assert_eq!(contexts[0].source, in_cluster.id, "never a raw pseudo path");
    assert!(
        sources
            .loaded()
            .unwrap()
            .is_in_cluster(&"in-cluster".into())
    );
    assert!(sources.diagnostics().contains(&Diagnostic::InClusterUsed));

    // Once a kubeconfig gives a context, the fallback steps aside.
    write(&default, &simple(&[("a", "https://a")]));
    let diff = sources.reload().await.unwrap();
    assert_eq!(names(&diff.added), ["a"]);
    assert_eq!(diff.removed, [in_cluster_cluster_id()]);
    assert!(
        sources
            .sources()
            .await
            .unwrap()
            .iter()
            .all(|s| s.kind != SourceKind::InCluster)
    );
}

#[tokio::test]
async fn in_a_pod_a_broken_kubeconfig_does_not_fall_back_to_in_cluster() {
    let dir = TempDir::new().unwrap();
    let default = default_in(dir.path());
    write(&default, "{{{ not yaml");
    let mut config = config_for(&default);
    pod_env(&mut config);
    let (sources, _) = adapter(config);

    assert!(sources.contexts().await.unwrap().is_empty());
    assert!(
        sources
            .diagnostics()
            .contains(&Diagnostic::InClusterSkipped {
                reason: InClusterSkip::BrokenKubeconfig
            })
    );
}

/// The wiring the module docs describe: on each `SourcesChanged`, hand the adapter's loader
/// result to `ClientPool::replace_loaded`. Only the edited context's client is dropped.
#[tokio::test]
async fn a_sources_changed_driven_pool_replace_drops_only_changed_contexts() {
    let dir = TempDir::new().unwrap();
    let path = default_in(dir.path());
    let servers = |b: &str| {
        simple(&[
            ("a", "https://127.0.0.1:1"),
            ("b", b),
            ("c", "https://127.0.0.3:1"),
        ])
    };
    write(&path, &servers("https://127.0.0.2:1"));
    let (sources, _) = adapter(config_for(&path));
    sources.reload().await.unwrap();
    let loaded = sources.loaded().unwrap();
    let pool = ClientPool::with_parts(
        loaded.merged.clone(),
        PoolConfig::default(),
        Arc::new(KubeClientFactory::new(ProxyEnv::default())),
        Arc::new(SystemClock),
    );
    assert!(pool.replace_loaded(&loaded).is_empty());
    for name in ["a", "b", "c"] {
        pool.get(&name.into()).await.expect("client builds offline");
    }
    let mut events = sources.subscribe();

    // Edit b's server, and touch the file without changing a or c.
    write(&path, &servers("https://127.0.0.9:1"));
    sources.reload().await.unwrap();
    let diff = try_next(&mut events).expect("a SourcesChanged for the edit");
    assert_eq!(ctx_names(&diff), (vec![], vec!["b".into()]));

    let dropped = pool.replace_loaded(&sources.loaded().unwrap());
    assert_eq!(dropped, [ContextName::new("b")]);
    assert!(pool.contains(&"a".into()) && pool.contains(&"c".into()));
    assert!(!pool.contains(&"b".into()));

    // Removing a context from the file drops its client too.
    write(&path, &simple(&[("a", "https://127.0.0.1:1")]));
    sources.reload().await.unwrap();
    let diff = try_next(&mut events).expect("a SourcesChanged for the removal");
    assert_eq!(diff.removed.len(), 2);
    assert_eq!(
        pool.replace_loaded(&sources.loaded().unwrap()),
        [ContextName::new("c")]
    );
    assert!(pool.contains(&"a".into()));
}

/// The UI's diff and the pool's invalidation share one notion of a context's connection: for
/// each edit, a context is `changed` exactly when its pool definition is no longer the same
/// connection.
#[tokio::test]
async fn the_diff_marks_changed_exactly_the_contexts_whose_pool_definition_changed() {
    let dir = TempDir::new().unwrap();
    let path = default_in(dir.path());
    let pair = [("a", "https://a"), ("b", "https://b")];
    let edits = [
        yaml(&pair, None, "token-1", None),
        yaml(
            &[("a", "https://a2"), ("b", "https://b")],
            None,
            "token-1",
            None,
        ),
        yaml(
            &[("a", "https://a2"), ("b", "https://b")],
            None,
            "token-1",
            None,
        ),
        yaml(
            &[("a", "https://a2"), ("b", "https://b")],
            None,
            "token-2",
            None,
        ),
        yaml(
            &[("a", "https://a2"), ("b", "https://b")],
            None,
            "token-2",
            Some("ns"),
        ),
        yaml(
            &[("a", "https://a2"), ("b", "https://b")],
            None,
            "token-2",
            Some("ns"),
        )
        .replace("    user: b\n", "    user: a\n"),
        yaml(
            &[("a", "https://a2"), ("b", "https://b")],
            None,
            "token-2",
            Some("ns"),
        )
        .replace("    user: b\n", "    user: a\n")
        .replace(
            "- name: b\n  user:\n    token: token-2",
            "- name: b\n  user:\n    token: unused",
        ),
    ];
    write(&path, &edits[0]);
    let (sources, _) = adapter(config_for(&path));
    sources.reload().await.unwrap();
    for (step, text) in edits.iter().enumerate().skip(1) {
        let before = sources.loaded().unwrap();
        write(&path, text);
        let diff = sources.reload().await.unwrap();
        let after = sources.loaded().unwrap();
        let expected: Vec<String> = ["a", "b"]
            .into_iter()
            .filter(|name| {
                let name = ContextName::new(*name);
                let old = ContextDefinition::from_kubeconfig(&before.merged, &name).unwrap();
                let new = ContextDefinition::from_kubeconfig(&after.merged, &name).unwrap();
                !old.same_connection(&new)
            })
            .map(str::to_owned)
            .collect();
        assert_eq!(names(&diff.changed), expected, "edit {step}");
    }
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
    let default = default_in(dir.path());
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
    let default = dir.path().join("no-home/.kube/config");
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
    let (sources, _) = adapter({
        let mut config = config_without_default();
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
    let (sources, secrets) = adapter(config_without_default());
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
    let (sources, secrets) = adapter(config_without_default());
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
    let default = default_in(dir.path());
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

/// Replace `target` with new content by atomic rename (staged next to it) every 250 ms until
/// `events` yields a diff, or fail after two seconds. Repeating the replacement (with different
/// content each time) tolerates a late first FSEvents delivery; the assertion is that a diff
/// arrives.
async fn replace_until_event(
    target: &Path,
    events: &mut BoxStream<'static, SourcesChanged>,
) -> SourcesChanged {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut attempt = 0;
    loop {
        attempt += 1;
        let staged = target.with_file_name(format!("config.new.{attempt}"));
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
    let path = default_in(dir.path());
    write(&path, &simple(&[("a", "https://a")]));
    let (sources, _) = adapter(watching_config(&path));
    sources.contexts().await.unwrap();
    let mut events = sources.subscribe();
    assert_eq!(sources.wait_for_watcher().await, WatchStatus::Active);

    let diff = replace_until_event(&path, &mut events).await;
    assert!(mentions_b(&diff), "{diff:?}");
}

/// `~/.kube/config -> elsewhere` (dotfile managers, Nix): writes land in the target's
/// directory, which must be watched too.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_watcher_sees_writes_through_a_symlinked_kubeconfig() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real");
    let links = dir.path().join("links").join(".kube");
    fs::create_dir(&real).unwrap();
    fs::create_dir_all(&links).unwrap();
    let target = real.join("config");
    write(&target, &simple(&[("a", "https://a")]));
    let link = links.join("config");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let (sources, _) = adapter(watching_config(&link));
    assert_eq!(names(&sources.contexts().await.unwrap()), ["a"]);
    let mut events = sources.subscribe();
    assert_eq!(sources.wait_for_watcher().await, WatchStatus::Active);

    let diff = replace_until_event(&target, &mut events).await;
    assert!(mentions_b(&diff), "{diff:?}");
}

/// A burst of writes inside the debounce window becomes one reload that sees the final state.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_burst_of_writes_is_coalesced_into_one_reload() {
    let dir = TempDir::new().unwrap();
    let path = default_in(dir.path());
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
    let path = dir.path().join("not-yet").join(".kube").join("config");
    let mut config = watching_config(&path);
    config.poll_interval = Duration::from_millis(100);
    let (sources, _) = adapter(config);
    assert!(sources.contexts().await.unwrap().is_empty());
    let mut events = sources.subscribe();
    assert!(matches!(
        sources.wait_for_watcher().await,
        WatchStatus::Failed(_)
    ));

    fs::create_dir_all(path.parent().unwrap()).unwrap();
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
    let mut config = config_for(&default_in(dir.path()));
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

/// Timings from settings are checked up front. A zero poll interval used to make the watch
/// task panic after it had reported `Active`, so nothing was watched while the status said
/// otherwise.
#[tokio::test]
async fn out_of_range_watch_timings_are_a_validation_error() {
    let dir = TempDir::new().unwrap();
    let path = default_in(dir.path());
    let invalid = [
        (Duration::ZERO, Duration::from_millis(300)),
        (MAX_POLL_INTERVAL + Duration::from_secs(1), Duration::ZERO),
        (Duration::MAX, Duration::ZERO),
        (
            Duration::from_secs(60),
            MAX_DEBOUNCE + Duration::from_millis(1),
        ),
        (Duration::from_secs(60), Duration::MAX),
    ];
    for (poll_interval, debounce) in invalid {
        let mut config = watching_config(&path);
        config.poll_interval = poll_interval;
        config.debounce = debounce;
        let err = KubeconfigSources::new(config, Arc::new(FakeSecretStorePort::new()))
            .expect_err("rejected");
        assert_eq!(
            err.kind(),
            ErrorKind::Validation,
            "{poll_interval:?} {debounce:?}"
        );
    }
    let mut config = watching_config(&path);
    config.poll_interval = MAX_POLL_INTERVAL;
    config.debounce = Duration::ZERO;
    let (sources, _) = adapter(config);
    assert_eq!(sources.wait_for_watcher().await, WatchStatus::Active);
}

/// A change made after the first load read the files but before the watches were registered
/// produces no event. The watch task re-reads once after registering, before it reports
/// `Active`, so the change is not left to the 60 s poll.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_before_the_watches_are_registered_is_caught_up() {
    let dir = TempDir::new().unwrap();
    let path = default_in(dir.path());
    write(&path, &simple(&[("a", "https://a")]));
    // First load with no watcher yet, then the change, then the watch task starts: the
    // ordering a startup race produces, made deterministic.
    let (sources, _) = adapter(config_for(&path));
    assert_eq!(names(&sources.contexts().await.unwrap()), ["a"]);
    write(&path, &simple(&[("a", "https://a"), ("b", "https://b")]));
    let mut events = sources.subscribe();
    sources
        .inner
        .watch_status
        .send_replace(WatchStatus::Starting);
    let _guard = watcher::spawn(Arc::downgrade(&sources.inner), &watching_config(&path)).unwrap();
    assert_eq!(sources.wait_for_watcher().await, WatchStatus::Active);

    let diff = tokio::time::timeout(Duration::from_secs(1), events.next())
        .await
        .expect("the catch-up reload reports the change")
        .unwrap();
    assert_eq!(names(&diff.added), ["b"]);
}
