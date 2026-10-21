//! The Helm page: native-only mode hides install/upgrade/rollback/uninstall/repos/charts and
//! keeps the release views; full mode shows everything.

use gpui::TestAppContext;
use oxikube_domain::helm::{HelmCapabilities, HelmFeature};

use super::{Harness, full};

fn row(feature: HelmFeature) -> &'static str {
    Box::leak(format!("helm-feature-{}", feature.name()).into_boxed_str())
}

#[gpui::test]
fn native_only_mode_hides_the_cli_features_and_keeps_the_releases(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, HelmCapabilities::none());
    for feature in HelmFeature::ALL {
        assert_eq!(
            h.is_laid_out(row(feature)),
            !feature.needs_cli(),
            "{feature:?}"
        );
    }
    for hidden in [
        "install",
        "upgrade",
        "rollback",
        "uninstall",
        "charts",
        "repos",
    ] {
        assert!(!h.is_laid_out(Box::leak(format!("helm-feature-{hidden}").into_boxed_str())));
    }
    assert!(h.is_laid_out("helm-banner"), "with guidance");
}

#[gpui::test]
fn full_mode_shows_every_feature_and_no_banner(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, full());
    for feature in HelmFeature::ALL {
        assert!(h.is_laid_out(row(feature)), "{feature:?}");
    }
    assert!(!h.is_laid_out("helm-banner"));
}

#[gpui::test]
fn the_page_follows_hot_detection(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, HelmCapabilities::none());
    assert!(!h.is_laid_out(row(HelmFeature::Install)));
    h.detected(full());
    assert!(h.is_laid_out(row(HelmFeature::Install)), "helm appeared");
    h.detected(HelmCapabilities::none());
    assert!(!h.is_laid_out(row(HelmFeature::Install)), "helm went away");
    assert!(h.is_laid_out(row(HelmFeature::Releases)));
}
