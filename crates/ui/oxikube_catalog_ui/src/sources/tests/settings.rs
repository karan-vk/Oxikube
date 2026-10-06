//! The settings side: `kubeconfig.sources` in `settings.json` is the stored list, and a change
//! of it, by the screen or by hand, reaches the cluster source.

use std::sync::Arc;

use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_app::{KubeconfigSourcesService, SourceListStore};
use oxikube_domain::command::NewKubeconfigSource;
use oxikube_ports::UserSource;
use oxikube_settings::SettingsStore;
use oxikube_testkit::{FakeClusterSourcePort, FakeFsPort};

use super::super::settings::{SettingsSourceList, follow};
use super::{DIR, kubeconfig};

/// A settings store over a temp config dir holding `user_settings`, and the list over it.
struct Env {
    dir: tempfile::TempDir,
    handle: super::super::settings::SettingsSourceListHandle,
}

impl Env {
    fn new(cx: &mut TestAppContext, user_settings: &str) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("settings.json"), user_settings).unwrap();
        let handle = cx.update(|cx| {
            oxikube_runtime::init_deterministic(cx);
            oxikube_settings::init_with_dir(dir.path(), cx);
            SettingsSourceList::install(cx)
        });
        Self { dir, handle }
    }

    fn file_text(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("settings.json")).unwrap()
    }
}

#[gpui::test]
async fn the_list_starts_as_the_shipped_default_and_a_save_writes_settings_json(
    cx: &mut TestAppContext,
) {
    let env = Env::new(cx, "{\n  // my own note\n  \"ui_scale\": 1.25\n}\n");
    let store = env.handle.store();
    assert_eq!(
        store.load().await.unwrap(),
        vec![UserSource::default_source()],
        "default.json lists kubectl's files"
    );

    let wanted = vec![
        UserSource::default_source(),
        UserSource::file("/config/kubeconfigs/prod.yaml"),
        UserSource::dir("/work/configs"),
    ];
    let saving = cx.background_executor.spawn({
        let store = store.clone();
        let wanted = wanted.clone();
        async move { store.save(&wanted).await }
    });
    cx.run_until_parked();
    saving.await.unwrap();

    assert_eq!(
        store.load().await.unwrap(),
        wanted,
        "the mirror follows the write"
    );
    let text = env.file_text();
    assert!(
        text.contains("// my own note"),
        "the user's comment survives: {text}"
    );
    assert!(text.contains("\"ui_scale\": 1.25"), "{text}");
    assert!(text.contains("/config/kubeconfigs/prod.yaml"), "{text}");
    assert!(text.contains("\"kind\": \"dir\""), "{text}");
    // Paths only: nothing but locations is written.
    assert!(!text.contains("token"), "{text}");
}

#[gpui::test]
async fn the_service_over_settings_writes_the_list_and_tells_the_cluster_source(
    cx: &mut TestAppContext,
) {
    let env = Env::new(cx, "{}");
    let source = Arc::new(FakeClusterSourcePort::new());
    let fs = Arc::new(FakeFsPort::new());
    let service =
        KubeconfigSourcesService::new(source.clone(), fs.clone(), env.handle.store(), DIR.into());
    let adding = cx.background_executor.spawn({
        let service = service.clone();
        async move {
            service
                .add(&NewKubeconfigSource::Pasted {
                    name: "prod".into(),
                    text: oxikube_domain::command::PastedText::new(kubeconfig(1)),
                })
                .await
        }
    });
    cx.run_until_parked();
    adding.await.unwrap();

    let text = env.file_text();
    assert!(text.contains("/config/kubeconfigs/prod.yaml"), "{text}");
    assert_eq!(
        source.user_sources(),
        vec![
            UserSource::default_source(),
            UserSource::file("/config/kubeconfigs/prod.yaml")
        ]
    );
    assert!(fs.is_private("/config/kubeconfigs/prod.yaml"));
}

#[gpui::test]
fn editing_settings_json_by_hand_reaches_the_cluster_source(cx: &mut TestAppContext) {
    let env = Env::new(cx, "{}");
    let source = Arc::new(FakeClusterSourcePort::new());
    let service = KubeconfigSourcesService::new(
        source.clone(),
        Arc::new(FakeFsPort::new()),
        env.handle.store(),
        DIR.into(),
    );
    let _follow = cx.update(|cx| follow(service, cx));
    cx.run_until_parked();
    assert_eq!(
        source.user_sources(),
        vec![UserSource::default_source()],
        "the shipped list is applied at start"
    );

    // The hot reload applies the edited text; no restart.
    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| {
            store
                .set_user_settings(
                    r#"{ "kubeconfig": { "sources": [
                        { "kind": "dir", "path": "/work/configs" },
                        { "kind": "file", "path": "/work/a.yaml" }
                    ] } }"#,
                )
                .unwrap();
        });
    });
    cx.run_until_parked();
    assert_eq!(
        source.user_sources(),
        vec![
            UserSource::dir("/work/configs"),
            UserSource::file("/work/a.yaml")
        ]
    );
}

#[gpui::test]
async fn without_a_settings_store_the_list_is_the_default_and_saving_says_why_it_cannot(
    cx: &mut TestAppContext,
) {
    let handle = cx.update(|cx| {
        oxikube_runtime::init_deterministic(cx);
        SettingsSourceList::install(cx)
    });
    let store = handle.store();
    assert_eq!(
        store.load().await.unwrap(),
        vec![UserSource::default_source()]
    );
    let saving = cx.background_executor.spawn({
        let store = store.clone();
        async move { store.save(&[]).await }
    });
    cx.run_until_parked();
    let error = saving.await.unwrap_err();
    assert_eq!(error.kind(), oxikube_domain::ErrorKind::Unsupported);
}
