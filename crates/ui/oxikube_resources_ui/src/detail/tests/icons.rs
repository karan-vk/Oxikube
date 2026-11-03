//! The header's kind and status icons follow the icon theme (E22-S11): Lucide by default, the
//! theme's `kind:Pod` and `status:Running` once it is selected.

use std::path::Path;

use gpui::{TestAppContext, UpdateGlobal as _};
use oxikube_settings::SettingsStore;
use oxikube_ui::IconName;

use super::fixture::{Detail, pod_ref, web_pod, web_replicaset};
use crate::detail::DetailView;
use crate::detail::render::status_icon;

/// Installs settings, themes and the test icon themes of `oxikube_theme`, Lucide selected.
fn install_icon_themes(d: &mut Detail) {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../platform/oxikube_theme/tests/fixtures/icon_themes");
    d.f.vcx.update(|_, cx| {
        let store = SettingsStore::new(oxikube_assets::default_settings())
            .expect("the default settings parse");
        cx.set_global(store);
        oxikube_theme::init_with_dir(None, cx);
        oxikube_theme::icons::init_icon_themes_with_dir(Some(&fixtures), cx);
    });
}

fn select_icon_theme(d: &mut Detail, user_settings: &str) {
    d.f.vcx.update(|_, cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(user_settings)
                .expect("the test settings parse");
        });
    });
    d.settle();
}

/// The paths of the header's kind icon and status icon.
fn header_icons(d: &mut Detail, view: &gpui::Entity<DetailView>) -> (String, Option<String>) {
    d.read(view, |v| {
        let status = v
            .model()
            .and_then(|model| model.header.status.as_ref())
            .map(|chip| status_icon(chip).path().to_string());
        (v.kind_icon().path().to_string(), status)
    })
}

#[gpui::test]
fn the_header_draws_the_kind_and_status_icons_of_the_icon_theme(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod(), web_replicaset()]);
    install_icon_themes(&mut d);
    let view = d.open(&pod_ref("web-0"));
    assert!(d.shown("detail-kind-icon"), "the kind chip has its icon");
    assert!(
        d.shown("detail-status-icon"),
        "the status chip has its icon"
    );

    let (kind, status) = header_icons(&mut d, &view);
    assert_eq!(kind, IconName::Box.path(), "Lucide's pod icon");
    assert_eq!(
        status.as_deref(),
        Some(IconName::CircleCheck.path()),
        "Lucide's icon for Running"
    );

    select_icon_theme(&mut d, r#"{ "icon_theme": "Kube Test Icons" }"#);
    let (kind, status) = header_icons(&mut d, &view);
    assert!(
        kind.ends_with("/kube/pod.svg"),
        "the theme's kind:Pod: {kind}"
    );
    let status = status.expect("a Running pod has a status chip");
    assert!(
        status.ends_with("/kube/running.svg"),
        "the theme's status:Running: {status}"
    );
    assert!(
        d.shown("detail-status-icon"),
        "still drawn after the switch"
    );

    // Back to Lucide, which also resets the paths this thread draws with for later tests.
    select_icon_theme(&mut d, "{}");
    let (kind, _) = header_icons(&mut d, &view);
    assert_eq!(kind, IconName::Box.path());
}
