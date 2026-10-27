//! The `cloud` settings reaching the discovery service and the CLI runner live (E18-S07), with
//! fake binaries on disk and fakes behind the service.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{BorrowAppContext as _, TestAppContext};
use jiff::Timestamp;
use oxikube_app::cloud::DiscoveryDeps;
use oxikube_app::session::ClusterSessionManager;
use oxikube_app::sources::{KubeconfigSourcesService, MemorySourceList};
use oxikube_app::store::Spawner;
use oxikube_ports::{CloudProvider, UserSource};
use oxikube_settings::{CloudSettings, SettingsStore};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeFsPort, FakeStatePort,
};

use super::*;

fn fake_cli(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn set_settings(cx: &mut TestAppContext, text: &str) {
    let text = text.to_owned();
    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| store.set_user_settings(&text).unwrap());
    });
}

fn deps() -> DiscoveryDeps {
    let source = Arc::new(FakeClusterSourcePort::new());
    let fs = Arc::new(FakeFsPort::new());
    let clock = Arc::new(FakeClockPort::new(
        Timestamp::from_second(1_800_000_000).unwrap(),
    ));
    let spawner: Arc<dyn Spawner> = Arc::new(|_| {});
    DiscoveryDeps {
        providers: Vec::new(),
        state: Arc::new(FakeStatePort::new()),
        fs: fs.clone(),
        sources: KubeconfigSourcesService::new(
            source.clone(),
            fs,
            Arc::new(MemorySourceList::new([UserSource::default_source()])),
            PathBuf::from("/config/kubeconfigs"),
        ),
        source: source.clone(),
        sessions: ClusterSessionManager::new(
            Arc::new(FakeClusterConnectorPort::new()),
            source,
            clock.clone(),
        ),
        clock,
        spawner,
    }
}

fn start(cx: &mut TestAppContext, settings: &str) -> CloudDiscoveryService {
    let text = settings.to_owned();
    cx.update(|cx| {
        let mut store =
            SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
        store.register_setting::<CloudSettings>();
        store.set_user_settings(&text).unwrap();
        cx.set_global(store);
        install(cx, deps())
    })
}

#[gpui::test]
fn the_defaults_enable_all_three_providers_with_the_default_ttl_and_fan_out(
    cx: &mut TestAppContext,
) {
    let service = start(cx, "{}");
    assert_eq!(
        service.providers(),
        [CloudProvider::Aws, CloudProvider::Gcp, CloudProvider::Azure]
    );
    let config = service.config();
    assert_eq!(config, DiscoveryConfig::default());
}

#[gpui::test]
fn editing_the_settings_reconfigures_the_service_without_a_restart(cx: &mut TestAppContext) {
    let service = start(cx, "{}");
    set_settings(
        cx,
        r#"{ "cloud": { "providers": { "gcloud": false }, "cache_ttl_secs": 60,
             "parallelism": 2 } }"#,
    );
    assert_eq!(
        service.providers(),
        [CloudProvider::Aws, CloudProvider::Azure],
        "the gcloud card is hidden at once"
    );
    let config = service.config();
    assert_eq!(config.ttl.as_secs(), 60);
    assert_eq!(config.max_parallel_scopes.get(), 2);
    assert!(config.disabled.contains(&CloudProvider::Gcp));

    set_settings(cx, "{}");
    assert_eq!(service.providers().len(), 3, "and shown again");
    assert_eq!(service.config(), DiscoveryConfig::default());
}

#[test]
fn settings_map_onto_the_runner_configuration() {
    let settings = CloudSettings {
        aws_path: Some("/opt/aws".into()),
        kubelogin_path: Some("/opt/kubelogin".into()),
        parallelism: 7,
        command_timeout: std::time::Duration::from_secs(90),
        ..CloudSettings::default()
    };
    let config = runner_config(&settings);
    assert_eq!(config.paths.len(), 2);
    assert_eq!(config.paths[&Cli::Aws], PathBuf::from("/opt/aws"));
    assert_eq!(
        config.paths[&Cli::Kubelogin],
        PathBuf::from("/opt/kubelogin")
    );
    assert_eq!(config.max_concurrent, 7);
    assert_eq!(config.default_timeout.as_secs(), 90);
    assert_eq!(service_config(&settings).max_parallel_scopes.get(), 7);
}

#[gpui::test]
fn a_changed_cli_path_reaches_the_runner(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let aws = fake_cli(dir.path(), "aws-custom");
    let runner = cx.update(|cx| {
        let mut store =
            SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
        store.register_setting::<CloudSettings>();
        cx.set_global(store);
        let settings = CloudSettings::resolve(cx);
        let runner = Arc::new(CliRunner::new(runner_config(&settings)));
        let adapters = Adapters {
            runner: runner.clone(),
            aws: Arc::new(AwsDiscovery::new(runner.clone(), aws_settings(&settings))),
            gcloud: Arc::new(GcloudDiscovery::new(runner.clone())),
            az: Arc::new(AzureAksAdapter::new(runner.clone())),
        };
        let service = CloudDiscoveryService::new(deps());

        CloudSettings::observe(cx, move |cx| {
            adapters.apply(&CloudSettings::resolve(cx), &service);
        })
        .detach();
        runner
    });
    set_settings(cx, &format!(r#"{{ "cloud": {{ "aws_path": {aws:?} }} }}"#));
    let located = futures::executor::block_on(oxikube_cloud::process::ProcessRunner::locate(
        runner.as_ref(),
        Cli::Aws,
    ));
    assert_eq!(located, Some(aws));
}
