//! Kind integration: rewriting the kubeconfig updates the context list and the pool
//! (E03-S09; sources and hot reload from E03-S02, pool from E03-S03). Needs
//! `cargo xtask kind-up` and `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::stream::BoxStream;
use kube::config::{Kubeconfig, NamedContext};
use oxikube_domain::ids::ContextName;
use oxikube_kube::kubeconfig::{Env, Platform};
use oxikube_kube::sources::{KubeconfigSources, SourcesConfig, WatchStatus};
use oxikube_ports::{ClusterSourcePort, SourcesChanged};
use oxikube_testkit::FakeSecretStorePort;

use common::whoami;

/// The time the epic allows between the file change and the new context list.
const RELOAD_BUDGET: Duration = Duration::from_secs(2);

/// The kind kubeconfig plus a copy of its context named `name`, defaulting to `namespace`.
fn with_alias(kubeconfig: &Kubeconfig, name: &str, namespace: &str) -> Kubeconfig {
    let mut kubeconfig = kubeconfig.clone();
    let mut alias: NamedContext = kubeconfig.contexts[0].clone();
    alias.name = name.to_owned();
    if let Some(body) = alias.context.as_mut() {
        body.namespace = Some(namespace.to_owned());
    }
    kubeconfig.contexts.push(alias);
    kubeconfig
}

/// Writes `kubeconfig` to `target` by atomic rename from a file staged next to it, as
/// editors and `kubectl config` do. JSON is valid YAML, so the loader reads it as is.
/// The file holds the kind admin's credentials: owner-only, inside the test's temp dir.
fn replace(target: &Path, kubeconfig: &Kubeconfig) {
    let staged = target.with_file_name("config.staged");
    fs::write(&staged, serde_json::to_vec(kubeconfig).expect("serialise")).expect("stage");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    fs::rename(&staged, target).expect("rename over the kubeconfig");
}

/// Applies `kubeconfig` and waits for the diff. The replacement is repeated every 250 ms
/// (same content) in case the first file event is delivered late, as FSEvents can be
/// for a freshly watched directory; the budget counts from the first rename.
async fn replace_until_changed(
    target: &Path,
    kubeconfig: &Kubeconfig,
    events: &mut BoxStream<'static, SourcesChanged>,
) -> (SourcesChanged, Duration) {
    let started = Instant::now();
    loop {
        replace(target, kubeconfig);
        let remaining = RELOAD_BUDGET.saturating_sub(started.elapsed());
        let wait = remaining.min(Duration::from_millis(250));
        if let Ok(Some(diff)) = tokio::time::timeout(wait, events.next()).await {
            return (diff, started.elapsed());
        }
        assert!(
            started.elapsed() < RELOAD_BUDGET,
            "no SourcesChanged within {RELOAD_BUDGET:?}"
        );
    }
}

fn names<'a>(
    contexts: impl IntoIterator<Item = &'a oxikube_ports::ClusterContext>,
) -> Vec<&'a str> {
    contexts.into_iter().map(|c| c.context.as_str()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kubeconfig_rewrite_emits_diff() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("config");
    let edited = ContextName::from("oxi-reload-edited");
    let added = ContextName::from("oxi-reload-added");
    let before = with_alias(&kind.kubeconfig, edited.as_str(), "default");
    replace(&path, &before);

    // KUBECONFIG names the temp file alone: the developer's kubeconfig is never read.
    let mut config = SourcesConfig::new(Env {
        platform: Platform::host(),
        kubeconfig: Some(path.clone().into_os_string()),
        ..Env::default()
    });
    config.debounce = Duration::from_millis(50);
    let sources = KubeconfigSources::new(config, Arc::new(FakeSecretStorePort::new()))
        .expect("sources with watcher");
    let contexts = sources.contexts().await.expect("first load");
    assert_eq!(names(&contexts), [kind.context.as_str(), edited.as_str()]);
    let mut events = sources.subscribe();
    assert_eq!(sources.wait_for_watcher().await, WatchStatus::Active);

    // The pool, driven from the adapter's loader result, with both contexts connected.
    let pool = kind.pool(Kubeconfig::default());
    assert!(
        pool.replace_loaded(&sources.loaded().expect("loaded"))
            .is_empty()
    );
    let kind_client = pool.get(&kind.context).await.expect("kind client");
    let edited_client = pool.get(&edited).await.expect("edited client");

    // Rewrite: the alias moves to another namespace and a new context appears.
    let after = with_alias(
        &with_alias(&kind.kubeconfig, edited.as_str(), "kube-system"),
        added.as_str(),
        "default",
    );
    let (diff, took) = replace_until_changed(&path, &after, &mut events).await;
    eprintln!("hot reload: SourcesChanged {took:?} after the first rename");
    assert_eq!(names(&diff.added), [added.as_str()], "{diff:?}");
    assert_eq!(names(&diff.changed), [edited.as_str()], "{diff:?}");
    assert!(diff.removed.is_empty(), "{diff:?}");
    assert_eq!(
        diff.changed[0].default_namespace.as_deref(),
        Some("kube-system")
    );

    // Only the edited context's client is dropped; the kind client is kept.
    let dropped = pool.replace_loaded(&sources.loaded().expect("reloaded"));
    assert_eq!(dropped, std::slice::from_ref(&edited));
    assert!(Arc::ptr_eq(
        &kind_client,
        &pool.get(&kind.context).await.expect("kept")
    ));
    let rebuilt = pool.get(&edited).await.expect("rebuilt");
    assert!(!Arc::ptr_eq(&edited_client, &rebuilt));

    // The rebuilt and the new context both reach the cluster.
    for (name, client) in [
        (&edited, rebuilt),
        (&added, pool.get(&added).await.expect("added client")),
    ] {
        let user = whoami(&client)
            .await
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!user.is_empty(), "{name}");
    }
}
