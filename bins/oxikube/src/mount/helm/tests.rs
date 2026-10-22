//! The controller on the real Tokio runtime with a fake `helm` executable: first detection at
//! start-up, hot reload of `helm.path`, `helm::Recheck`, and the version gate.

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_domain::helm::{GateReason, HelmGate, HelmVersion};
use oxikube_helm_ui::HelmGateState;
use oxikube_settings::SettingsStore;

use super::{Request, install};

/// A shell script that answers `helm version --short` with `version`.
fn fake_helm(dir: &Path, name: &str, version: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\necho '{version}'\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn set_settings(cx: &mut TestAppContext, text: &str) {
    let text = text.to_owned();
    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| store.set_user_settings(&text).unwrap());
    });
}

fn gate(cx: &mut TestAppContext) -> HelmGate {
    cx.update(|cx| HelmGateState::current(cx))
}

/// Lets the controller and its child process finish (the runtime is real, so this polls).
fn settle(cx: &mut TestAppContext, until: impl Fn(&HelmGate) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if until(&gate(cx)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the gate never settled: {:?}",
            gate(cx)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn start(cx: &mut TestAppContext, settings: &str) -> tokio::runtime::Runtime {
    cx.executor().allow_parking();
    let runtime = oxikube_runtime::build_runtime().expect("a tokio runtime");
    let handle = runtime.handle().clone();
    let text = settings.to_owned();
    cx.update(|cx| {
        oxikube_runtime::init_from_handle(cx, handle);
        let mut store =
            SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
        store.register_setting::<oxikube_settings::HelmSettings>();
        store.set_user_settings(&text).unwrap();
        cx.set_global(store);
        install(cx);
    });
    runtime
}

#[gpui::test]
fn start_up_detects_the_configured_binary_in_the_background(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let helm = fake_helm(dir.path(), "helm", "v3.14.2+gc309b6f");
    let _rt = start(cx, &format!(r#"{{ "helm": {{ "path": {:?} }} }}"#, helm));
    settle(cx, |g| *g == HelmGate::Full);
    let caps = cx.update(|cx| HelmGateState::global(cx).unwrap().read(cx).capabilities());
    assert_eq!(caps.cli_version, Some(HelmVersion::new(3, 14, 2)));
}

#[gpui::test]
fn changing_helm_path_redetects_without_a_restart(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let good = fake_helm(dir.path(), "helm-good", "v3.14.2+gc309b6f");
    let old = fake_helm(dir.path(), "helm-old", "v3.8.1+g5cb9af4");
    let missing = dir.path().join("no-such-helm");
    let _rt = start(cx, &format!(r#"{{ "helm": {{ "path": {missing:?} }} }}"#));
    settle(cx, |g| {
        *g == HelmGate::NativeOnly {
            reason: GateReason::BinaryMissing,
        }
    });

    set_settings(cx, &format!(r#"{{ "helm": {{ "path": {good:?} }} }}"#));
    settle(cx, |g| *g == HelmGate::Full);

    set_settings(cx, &format!(r#"{{ "helm": {{ "path": {old:?} }} }}"#));
    settle(cx, |g| {
        matches!(
            g,
            HelmGate::NativeOnly {
                reason: GateReason::VersionTooOld { found, needed }
            } if found == "3.8.1" && needed == "3.10.0"
        )
    });

    set_settings(cx, &format!(r#"{{ "helm": {{ "path": {good:?} }} }}"#));
    settle(cx, |g| *g == HelmGate::Full);
}

#[gpui::test]
fn other_helm_keys_do_not_redetect_and_recheck_does(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let helm = fake_helm(dir.path(), "helm", "v3.14.2+gc309b6f");
    let _rt = start(cx, &format!(r#"{{ "helm": {{ "path": {helm:?} }} }}"#));
    settle(cx, |g| *g == HelmGate::Full);

    // Remove the binary behind the controller's back. A settings edit that leaves `path` alone
    // must not look again...
    std::fs::remove_file(&helm).unwrap();
    set_settings(
        cx,
        &format!(r#"{{ "helm": {{ "path": {helm:?}, "server_side_apply": true }} }}"#),
    );
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(gate(cx), HelmGate::Full, "no re-detection for another key");

    // ...but `helm::Recheck` always does.
    let sink = cx.update(install);
    sink.unbounded_send(Request::Recheck).unwrap();
    settle(cx, |g| *g != HelmGate::Full);
}

#[gpui::test]
fn the_check_in_flight_shows_and_ends(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let helm = fake_helm(dir.path(), "helm", "v3.14.2+gc309b6f");
    let _rt = start(cx, &format!(r#"{{ "helm": {{ "path": {helm:?} }} }}"#));
    settle(cx, |g| *g == HelmGate::Full);
    let checking = cx.update(|cx| HelmGateState::global(cx).unwrap().read(cx).is_checking());
    assert!(!checking, "a finished detection is not 'checking'");
}
