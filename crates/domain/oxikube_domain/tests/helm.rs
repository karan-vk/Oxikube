//! Helm domain helpers: status parsing, `Values` get/merge, `ReleaseRef` display, version
//! gate and the error mapping.

use oxikube_domain::ErrorKind;
use oxikube_domain::OxiError;
use oxikube_domain::helm::{
    ChartRef, HelmCapabilities, HelmError, HelmVersion, ReleaseRef, ReleaseStatus, Values,
};
use oxikube_domain::ids::{ClusterId, ContextName};

fn values(yaml: &str) -> Values {
    Values::from_yaml(yaml).unwrap()
}

#[test]
fn status_round_trips_every_variant_and_unknown_falls_back() {
    for status in ReleaseStatus::ALL {
        assert_eq!(ReleaseStatus::parse(status.as_str()), status);
    }
    assert_eq!(
        ReleaseStatus::parse("Pending-Install"),
        ReleaseStatus::PendingInstall
    );
    assert_eq!(
        ReleaseStatus::parse("from-the-future"),
        ReleaseStatus::Unknown
    );
    assert!(ReleaseStatus::PendingUpgrade.is_pending());
    assert!(!ReleaseStatus::Deployed.is_pending());
}

#[test]
fn release_ref_and_chart_ref_display() {
    let r = ReleaseRef::new(
        ClusterId::new("kubeconfig-a", &ContextName::new("kind-a")),
        "demo",
        "web",
    );
    assert_eq!(r.to_string(), "demo/web");
    let chart = ChartRef::new("nginx", "15.4.0").in_repo("bitnami");
    assert_eq!(chart.to_string(), "nginx-15.4.0");
    assert_eq!(chart.reference(), "bitnami/nginx");
    assert_eq!(ChartRef::new("nginx", "").to_string(), "nginx");
}

#[test]
fn values_get_walks_maps_and_lists() {
    let v = values("image:\n  tag: '1.2'\nhosts: [a, b]\n");
    assert_eq!(v.get("image.tag").and_then(|x| x.as_str()), Some("1.2"));
    assert_eq!(v.get("hosts.1").and_then(|x| x.as_str()), Some("b"));
    assert!(v.get("image.nope").is_none());
    assert!(v.get("hosts.x").is_none());
    assert_eq!(v.keys().collect::<Vec<_>>(), ["image", "hosts"]);
}

#[test]
fn values_merge_follows_helm_coalescing() {
    let base = values("a: 1\nm:\n  x: 1\n  y: 2\nl: [1, 2]\ngone: 1\n");
    let over = values("m:\n  y: 3\n  z: 4\nl: [9]\ngone: null\nnew:\n  k: null\n  v: 1\n");
    let merged = Values::merge(&base, &over);
    assert_eq!(merged.get("a").and_then(|x| x.as_i64()), Some(1));
    assert_eq!(merged.get("m.x").and_then(|x| x.as_i64()), Some(1));
    assert_eq!(merged.get("m.y").and_then(|x| x.as_i64()), Some(3));
    assert_eq!(merged.get("m.z").and_then(|x| x.as_i64()), Some(4));
    assert_eq!(merged.get("l.0").and_then(|x| x.as_i64()), Some(9));
    assert!(merged.get("l.1").is_none());
    assert!(merged.get("gone").is_none());
    assert!(merged.get("new.k").is_none());
    assert_eq!(merged.get("new.v").and_then(|x| x.as_i64()), Some(1));
    // inputs untouched
    assert_eq!(base.get("m.y").and_then(|x| x.as_i64()), Some(2));
}

#[test]
fn values_yaml_round_trip_empty_and_invalid() {
    assert!(Values::from_yaml("").unwrap().is_empty());
    assert!(Values::from_yaml("# only a comment\n").unwrap().is_empty());
    let v = values("a:\n  b: [1, 2]\n");
    assert_eq!(Values::from_yaml(&v.to_yaml().unwrap()).unwrap(), v);
    assert!(matches!(
        Values::from_yaml("a: [unclosed"),
        Err(HelmError::Parse(_))
    ));
}

#[test]
fn values_debug_never_prints_the_content() {
    let v = values("password: hunter2\n");
    let debug = format!("{v:?}");
    assert!(!debug.contains("hunter2"), "{debug}");
}

#[test]
fn version_parse_and_minimum() {
    assert_eq!(
        HelmVersion::parse("v3.14.2"),
        Some(HelmVersion::new(3, 14, 2))
    );
    assert_eq!(
        HelmVersion::parse("3.15.0-rc.1+gabc"),
        Some(HelmVersion::new(3, 15, 0))
    );
    assert_eq!(HelmVersion::parse("3.10"), Some(HelmVersion::new(3, 10, 0)));
    assert_eq!(HelmVersion::parse("garbage"), None);
    assert!(HelmVersion::new(3, 10, 0).is_supported());
    assert!(HelmVersion::new(4, 0, 0).is_supported());
    assert!(!HelmVersion::new(3, 9, 9).is_supported());
}

#[test]
fn capabilities_presets() {
    let none = HelmCapabilities::none();
    assert!(!none.native_read && !none.can_mutate());
    let native = HelmCapabilities::native_only();
    assert!(native.native_read && !native.cli_available && !native.can_mutate());
    let full = HelmCapabilities::full(HelmVersion::new(3, 14, 0));
    assert!(full.can_mutate() && full.can_search_repos && full.cli_available);
}

#[test]
fn errors_map_onto_the_port_taxonomy() {
    let kind = |e: HelmError| OxiError::from(e).kind();
    assert_eq!(kind(HelmError::Unsupported), ErrorKind::Unsupported);
    assert_eq!(kind(HelmError::BinaryMissing), ErrorKind::Unsupported);
    assert_eq!(
        kind(HelmError::VersionTooOld {
            found: "3.9.0".into(),
            needed: "3.10.0".into()
        }),
        ErrorKind::Unsupported
    );
    assert_eq!(
        kind(HelmError::ReleaseNotFound("x".into())),
        ErrorKind::NotFound
    );
    assert_eq!(kind(HelmError::Forbidden("x".into())), ErrorKind::Forbidden);
    assert_eq!(kind(HelmError::Timeout("5m".into())), ErrorKind::Timeout);
    assert_eq!(
        kind(HelmError::CliFailed {
            exit_code: Some(1),
            stderr_summary: "boom".into()
        }),
        ErrorKind::Internal
    );
    assert_eq!(kind(HelmError::Parse("x".into())), ErrorKind::Validation);
}
