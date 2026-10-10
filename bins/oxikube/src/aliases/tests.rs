//! The alias wiring over the real init order (deterministic runtime, no watcher threads): the
//! registry the jump bar reads is the one `aliases.json` feeds.

use gpui::TestAppContext;
use oxikube_app::{AliasSource, Resolution};
use oxikube_domain::AliasTarget;
use oxikube_domain::ids::{ClusterId, ContextName, Gvr};

use super::AliasWiring;
use crate::app_state::AppState;
use crate::startup::{ConfigSource, StartupEnv, init};

fn cluster() -> ClusterId {
    ClusterId::new("/home/me/.kube/config", &ContextName::new("dev"))
}

fn env_with_dir(dir: &std::path::Path) -> StartupEnv {
    let mut env = StartupEnv::test();
    env.config = ConfigSource::Dir(dir.to_path_buf());
    env
}

fn resolve(cx: &mut TestAppContext, word: &str) -> Resolution {
    cx.update(|cx| {
        AppState::global(cx)
            .services()
            .aliases
            .table(&cluster())
            .resolve(word)
    })
}

#[gpui::test]
async fn the_builtin_aliases_are_there_without_any_file(cx: &mut TestAppContext) {
    cx.update(|cx| init(cx, StartupEnv::test()).expect("init"));
    cx.run_until_parked();
    let Resolution::Exact(entry) = resolve(cx, "dp") else {
        panic!("dp should resolve");
    };
    assert_eq!(entry.source, AliasSource::BuiltIn);
    assert_eq!(
        entry.target,
        AliasTarget::Gvr(Gvr::new("apps", "v1", "deployments"))
    );
    // In-memory config reads no file.
    assert!(cx.update(|cx| cx.global::<AliasWiring>().file().is_none()));
}

#[gpui::test]
async fn the_users_file_is_read_at_start_up_and_bad_entries_are_reported_with_their_line(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("aliases.json"),
        "{\n  // mine\n  \"prodpods\": \"v1/pods\",\n  \"po\": \"v1/nodes\",\n  \"oops\": 7\n}\n",
    )
    .unwrap();
    cx.update(|cx| init(cx, env_with_dir(dir.path())).expect("init"));
    cx.run_until_parked();

    let Resolution::Exact(entry) = resolve(cx, "prodpods") else {
        panic!("the user alias should resolve");
    };
    assert_eq!(entry.source, AliasSource::User);
    // A user alias hides the built-in one of the same name.
    let Resolution::Exact(po) = resolve(cx, "PO") else {
        panic!("po should resolve");
    };
    assert_eq!(po.source, AliasSource::User);
    assert_eq!(po.target, AliasTarget::Gvr(Gvr::new("", "v1", "nodes")));

    let loaded = cx.update(|cx| cx.global::<AliasWiring>().file().expect("opened").loaded());
    assert_eq!(loaded.aliases.len(), 2);
    assert_eq!(loaded.diagnostics.len(), 1);
    assert_eq!(loaded.diagnostics[0].line, Some(5));
    assert_eq!(loaded.diagnostics[0].alias.as_deref(), Some("oops"));
}

#[gpui::test]
async fn a_reload_changes_what_the_jump_bar_resolves(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("aliases.json");
    std::fs::write(&path, r#"{"mine": "v1/pods"}"#).unwrap();
    cx.update(|cx| init(cx, env_with_dir(dir.path())).expect("init"));
    cx.run_until_parked();
    assert!(resolve(cx, "mine").is_known());

    // The watcher would call this with the new text when the file is saved.
    std::fs::write(&path, r#"{"yours": "apps/v1/deployments"}"#).unwrap();
    cx.update(|cx| {
        cx.global::<AliasWiring>()
            .file()
            .expect("opened")
            .reload_from_disk()
            .expect("reload");
    });
    assert!(!resolve(cx, "mine").is_known());
    assert!(resolve(cx, "yours").is_known());

    // A broken save keeps what was loaded.
    cx.update(|cx| {
        cx.global::<AliasWiring>()
            .file()
            .unwrap()
            .reload("{ \"yours\": ")
    });
    assert!(resolve(cx, "yours").is_known());
    let loaded = cx.update(|cx| cx.global::<AliasWiring>().file().unwrap().loaded());
    assert_eq!(loaded.diagnostics.len(), 1);
}

#[gpui::test]
async fn an_unreadable_file_leaves_the_builtins(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    // A directory where aliases.json should be: reading it fails.
    std::fs::create_dir(dir.path().join("aliases.json")).unwrap();
    cx.update(|cx| init(cx, env_with_dir(dir.path())).expect("init"));
    cx.run_until_parked();
    assert!(resolve(cx, "po").is_known());
    assert!(cx.update(|cx| cx.global::<AliasWiring>().file().is_none()));
}

#[gpui::test]
fn the_feature_is_part_of_the_real_start_up(_cx: &mut TestAppContext) {
    assert!(
        crate::startup::FEATURES
            .iter()
            .any(|feature| feature.name == "aliases")
    );
}
