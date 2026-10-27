//! Managed cloud kubeconfigs on a real filesystem (E18-S05): the file `CloudDiscoveryService`
//! writes through the real `FsPort` (`StdFs`) is owner-only, parses with the tolerant loader of
//! the real cluster source (`oxikube_kube::sources`, E03-S01) and appears in its contexts after the
//! reload; removing the last cluster takes the file and its contexts away again. The cloud
//! provider is the testkit fake: no CLI runs. No watcher: the service reloads by itself.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_app::cloud::{CloudDiscoveryService, DiscoveryDeps};
use oxikube_app::session::ClusterSessionManager;
use oxikube_app::sources::{KubeconfigSourcesService, MemorySourceList};
use oxikube_app::store::Spawner;
use oxikube_kube::kubeconfig::{Env, Platform};
use oxikube_kube::sources::{KubeconfigSources, SourcesConfig};
use oxikube_ports::{CloudProvider, CloudScope, ClusterSourcePort, DiscoveredCluster, UserSource};
use oxikube_runtime::StdFs;
use oxikube_testkit::{
    FakeClockPort, FakeCloudDiscoveryPort, FakeClusterConnectorPort, FakeSecretStorePort,
    FakeStatePort,
};

/// The real cluster source over a home without a kubeconfig, not watching.
fn cluster_source(home: &Path) -> Arc<KubeconfigSources> {
    let mut config = SourcesConfig::new(Env {
        platform: Platform::host(),
        home: Some(home.to_path_buf()),
        ..Env::default()
    });
    config.watch = false;
    Arc::new(
        KubeconfigSources::new(config, Arc::new(FakeSecretStorePort::new()))
            .expect("a configuration without a watcher is valid"),
    )
}

fn eks(name: &str) -> DiscoveredCluster {
    DiscoveredCluster {
        endpoint: Some(format!("https://{name}.gr7.eu-west-1.eks.amazonaws.com")),
        ca_data: Some("LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0t".into()),
        ..DiscoveredCluster::new(
            CloudScope::Aws {
                profile: "prod".into(),
                region: "eu-west-1".into(),
            },
            name,
            "eu-west-1",
        )
    }
}

fn context_names(contexts: &[oxikube_ports::ClusterContext]) -> Vec<String> {
    let mut names: Vec<_> = contexts.iter().map(|c| c.context.to_string()).collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn added_clusters_reach_the_real_cluster_source_and_leave_with_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let kubeconfigs = dir.path().join("config").join("kubeconfigs");
    let source = cluster_source(&dir.path().join("home"));
    let clock = Arc::new(FakeClockPort::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    ));
    let fs_port = Arc::new(StdFs::new());
    let sources = KubeconfigSourcesService::new(
        source.clone(),
        fs_port.clone(),
        Arc::new(MemorySourceList::new([UserSource::default_source()])),
        kubeconfigs.clone(),
    );
    let spawner: Arc<dyn Spawner> = Arc::new(|task| {
        tokio::spawn(task);
    });
    let (web, api) = (eks("web"), eks("api"));
    let port = Arc::new(
        FakeCloudDiscoveryPort::ready(CloudProvider::Aws)
            .with_scopes([web.scope.clone()])
            .with_clusters([web.clone(), api.clone()]),
    );
    let service = CloudDiscoveryService::new(DiscoveryDeps {
        providers: vec![port],
        state: Arc::new(FakeStatePort::new()),
        fs: fs_port,
        sources,
        source: source.clone(),
        sessions: ClusterSessionManager::new(
            Arc::new(FakeClusterConnectorPort::new()),
            source.clone(),
            clock.clone(),
        ),
        clock,
        spawner,
    });
    assert!(source.contexts().await.unwrap().is_empty());

    let web_ctx = service.add(&web).await.unwrap().context;
    let api_ctx = service.add(&api).await.unwrap().context;
    let path = kubeconfigs.join("aws.yaml");
    assert_eq!(service.managed_path(CloudProvider::Aws), path);

    // The cluster source read the file with the tolerant loader: both contexts, no problem,
    // their servers from the entries.
    let contexts = source.contexts().await.unwrap();
    assert_eq!(
        context_names(&contexts),
        [api_ctx.to_string(), web_ctx.to_string()]
    );
    for context in &contexts {
        assert_eq!(context.problem, None, "{context:?}");
        assert!(
            context
                .server
                .as_deref()
                .is_some_and(|s| s.ends_with(".eks.amazonaws.com")),
            "{context:?}"
        );
    }
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("interactiveMode: IfAvailable"), "{text}");
    assert!(!text.contains("token:"), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "owner-only");
    }
    // No temp file is left next to it.
    let names: Vec<_> = fs::read_dir(&kubeconfigs)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["aws.yaml"]);

    // Adding again changes nothing on disk.
    let before = fs::read(&path).unwrap();
    assert!(service.add(&web).await.unwrap().unchanged);
    assert_eq!(fs::read(&path).unwrap(), before);

    service.remove(CloudProvider::Aws, &web_ctx).await.unwrap();
    assert_eq!(
        context_names(&source.contexts().await.unwrap()),
        [api_ctx.to_string()]
    );
    let removed = service.remove(CloudProvider::Aws, &api_ctx).await.unwrap();
    assert!(removed.deleted_file);
    assert!(!path.exists());
    assert!(source.contexts().await.unwrap().is_empty());
}
