//! The banner: guidance with its two actions when something is hidden, nothing when all is well.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::helm::{HelmCapabilities, HelmVersion};

use super::{Harness, full};

#[gpui::test]
fn a_full_gate_shows_no_banner(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, full());
    assert!(!h.is_laid_out("helm-banner"));
}

#[gpui::test]
fn native_only_mode_shows_the_banner_with_locate_and_recheck(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, HelmCapabilities::none());
    assert!(h.is_laid_out("helm-banner"));
    assert!(h.is_laid_out("helm-locate"));
    assert!(h.is_laid_out("helm-recheck"));
    let text = h
        .state
        .read_with(&h.vcx, |s, _| s.gate().guidance().unwrap());
    assert!(text.contains("Install Helm") && text.contains("Settings (helm.path)"));
}

#[gpui::test]
fn the_buttons_send_the_helm_commands(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, HelmCapabilities::none());
    h.click("helm-locate");
    h.click("helm-recheck");
    assert_eq!(
        h.sent.sent(),
        vec![Command::HelmLocateBinary, Command::HelmRecheck]
    );
}

#[gpui::test]
fn the_banner_follows_a_recheck_that_finds_helm(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, HelmCapabilities::none());
    assert!(h.is_laid_out("helm-banner"));
    h.detected(full());
    assert!(!h.is_laid_out("helm-banner"), "found: the banner goes away");
    h.detected(HelmCapabilities {
        cli_version: Some(HelmVersion::new(3, 9, 0)),
        ..HelmCapabilities::none()
    });
    assert!(h.is_laid_out("helm-banner"), "too old: it comes back");
}

#[gpui::test]
fn a_recheck_in_flight_disables_the_button(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, HelmCapabilities::none());
    let state = h.state.clone();
    h.vcx
        .update(|_, cx| state.update(cx, |s, cx| s.begin_check(cx)));
    h.vcx.run_until_parked();
    h.click("helm-recheck");
    assert!(h.sent.sent().is_empty(), "a disabled button sends nothing");
    h.detected(HelmCapabilities::none());
    h.click("helm-recheck");
    assert_eq!(h.sent.sent(), vec![Command::HelmRecheck]);
}

#[gpui::test]
fn server_side_apply_on_an_old_helm_warns_in_the_banner(cx: &mut TestAppContext) {
    use gpui::UpdateGlobal as _;
    use oxikube_settings::SettingsStore;
    let mut h = Harness::open(cx, HelmCapabilities::full(HelmVersion::new(3, 12, 0)));
    assert!(!h.is_laid_out("helm-banner"));
    h.vcx.update(|_, cx| {
        let mut store =
            SettingsStore::without_registered(oxikube_assets::default_settings()).unwrap();
        store.register_setting::<oxikube_settings::HelmSettings>();
        store
            .set_user_settings(r#"{ "helm": { "server_side_apply": true } }"#)
            .unwrap();
        cx.set_global(store);
    });
    h.vcx.run_until_parked();
    assert!(h.is_laid_out("helm-banner"), "the warning shows");
    assert!(
        !h.is_laid_out("helm-locate"),
        "the binary is found: no Locate button"
    );
    h.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| store.set_user_settings("{}").unwrap());
    });
    h.vcx.run_until_parked();
    assert!(!h.is_laid_out("helm-banner"), "off again: gone");
}
