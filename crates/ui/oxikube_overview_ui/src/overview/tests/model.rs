//! The pure view-model: tile faces in every feed state, capacity lines and their text, top rows,
//! the metrics notice, and the info card's address with its credentials cut out.

use oxikube_app::MetricsState;
use oxikube_app::cluster_overview::{
    Distribution, FeedStatus, OverviewSnapshot, TopEntry, UsageTotals,
};
use oxikube_app::store::FeedState;
use oxikube_domain::metrics::MissingReason;
use oxikube_domain::{AllocationResource, ErrorKind};
use oxikube_ui::tile::TileTone;

use crate::overview::{
    VersionState, api_server_text, capacity_lines, health_tiles, info_rows, live_metrics_notice,
    top_rows,
};
use crate::test_support::live_snapshot;

fn tile<'a>(s: &'a [crate::overview::HealthTile], id: &str) -> &'a crate::overview::HealthTile {
    s.iter().find(|t| t.id == id).expect("the tile")
}

#[test]
fn tiles_count_nodes_and_pods_and_tone_what_needs_attention() {
    let tiles = health_tiles(&live_snapshot());
    let ready = tile(&tiles, "nodes-ready");
    assert_eq!(
        (ready.value.as_str(), ready.caption.as_str()),
        ("11", "of 12")
    );
    assert_eq!(ready.tone, TileTone::Good);
    let not_ready = tile(&tiles, "nodes-not-ready");
    assert_eq!(not_ready.value, "1");
    assert_eq!(not_ready.caption, "of 12 · 1 cordoned");
    assert_eq!(not_ready.tone, TileTone::Warn);
    assert_eq!(tile(&tiles, "pods-running").value, "312");
    assert_eq!(tile(&tiles, "pods-failed").tone, TileTone::Warn);
    assert_eq!(tile(&tiles, "pods-succeeded").tone, TileTone::Good);
    assert!(
        tiles.iter().all(|t| t.id != "pods-unknown"),
        "no unknown pods, no tile"
    );
}

#[test]
fn a_tile_without_numbers_says_loading_or_why() {
    let mut s = OverviewSnapshot::default();
    let tiles = health_tiles(&s);
    assert!(
        tiles
            .iter()
            .all(|t| t.value == "…" && t.tone == TileTone::Muted)
    );
    s.pods_feed = FeedStatus {
        state: FeedState::Forbidden {
            message: "pods is forbidden".into(),
        },
        partial: false,
    };
    s.nodes_feed = FeedStatus {
        state: FeedState::Failed {
            kind: ErrorKind::Network,
            message: "connection reset".into(),
        },
        partial: false,
    };
    let tiles = health_tiles(&s);
    let pods = tile(&tiles, "pods-running");
    assert_eq!(pods.value, "no access");
    assert!(
        pods.hover
            .as_deref()
            .is_some_and(|h| h.contains("forbidden"))
    );
    let nodes = tile(&tiles, "nodes-ready");
    assert_eq!(nodes.value, "–");
    assert_eq!(nodes.hover.as_deref(), Some("connection reset"));
}

#[test]
fn capacity_lines_join_allocatable_requests_limits_and_usage() {
    let [cpu, memory, pods] = capacity_lines(&live_snapshot());
    assert_eq!(cpu.resource, AllocationResource::Cpu);
    assert_eq!(cpu.allocatable, Some(48_000));
    assert_eq!(cpu.usage, Some(19_250));
    assert_eq!(cpu.requests_percent(), Some(65));
    assert!((cpu.requests_fill() - 0.65625).abs() < 1e-6);
    assert_eq!(
        cpu.limits_fill(),
        Some(1.0),
        "limits past allocatable are pinned"
    );
    let text = cpu.summary();
    assert!(text.starts_with("used "), "{text}");
    assert!(
        text.contains("(40%)") && text.contains("(65%)") && text.contains("(120%)"),
        "{text}"
    );
    assert!(memory.usage.is_some());
    assert_eq!(pods.limits, None);
    assert_eq!(pods.usage, None);
    assert!(
        pods.summary().contains("scheduled 321"),
        "{}",
        pods.summary()
    );
}

#[test]
fn without_usage_the_bars_say_nothing_about_it_and_never_zero() {
    let mut s = live_snapshot();
    s.usage = None;
    let [cpu, ..] = capacity_lines(&s);
    assert_eq!(cpu.usage, None);
    assert_eq!(cpu.usage_fill(), None);
    assert!(!cpu.summary().contains("used"));
    s.nodes_feed = FeedStatus::default();
    let [cpu, ..] = capacity_lines(&s);
    assert_eq!(cpu.allocatable, None, "no nodes listed: no denominator");
    assert_eq!(cpu.requests_fill(), 0.0);
}

#[test]
fn top_rows_name_pods_with_their_namespace_and_format_the_unit() {
    let pod = TopEntry {
        namespace: Some("shop".into()),
        name: "web-0".into(),
        value: 250_000_000,
    };
    let ns = TopEntry {
        namespace: None,
        name: "shop".into(),
        value: 512 * 1024 * 1024,
    };
    let cpu = top_rows(std::slice::from_ref(&pod), true);
    assert_eq!(cpu[0].label, "shop/web-0");
    assert_eq!(cpu[0].value, "250m");
    let memory = top_rows(&[ns], false);
    assert_eq!(memory[0].label, "shop");
    assert_eq!(memory[0].value, "512Mi");
    assert_eq!(memory[0].target, (None, "shop".to_owned()));
}

#[test]
fn the_notice_names_the_reason_metrics_are_missing() {
    let mut s = live_snapshot();
    assert_eq!(live_metrics_notice(&s, true), None);
    s.node_metrics = MetricsState::Unavailable(MissingReason::NotInstalled);
    let text = live_metrics_notice(&s, true).expect("a notice");
    assert!(
        text.starts_with("metrics unavailable: metrics-server is not installed"),
        "{text}"
    );
    assert!(live_metrics_notice(&OverviewSnapshot::default(), false).is_some());
    let mut loading = live_snapshot();
    loading.node_metrics = MetricsState::Loading;
    loading.usage = Some(UsageTotals::default());
    assert_eq!(
        live_metrics_notice(&loading, true),
        None,
        "loading is not absence"
    );
}

#[test]
fn the_api_server_is_shown_without_credentials() {
    assert_eq!(
        api_server_text("https://admin:s3cret@prod.example:6443/?token=abc"),
        "https://prod.example:6443/"
    );
    assert_eq!(
        api_server_text("https://k8s.example:443"),
        "https://k8s.example:443"
    );
    assert_eq!(
        api_server_text("https://k8s.example/clusters/a#x"),
        "https://k8s.example/clusters/a"
    );
    assert_eq!(api_server_text("user@10.0.0.1:6443"), "10.0.0.1:6443");
}

#[test]
fn the_info_rows_say_unknown_rather_than_guess() {
    let rows = info_rows(
        "prod",
        &VersionState::Loading,
        Distribution::Unknown,
        None,
        None,
    );
    assert_eq!(rows.version, "…");
    assert_eq!(rows.distribution, "unknown");
    assert_eq!(rows.nodes, "–");
    assert_eq!(rows.server, "–");
    let failed = VersionState::Failed("forbidden".into());
    assert_eq!(failed.text(), "unknown (forbidden)");
    let rows = info_rows(
        "prod",
        &VersionState::Known("v1.31.1+k3s1".into()),
        Distribution::K3s,
        Some(3),
        Some("https://10.0.0.1:6443"),
    );
    assert_eq!(
        (
            rows.version.as_str(),
            rows.distribution.as_str(),
            rows.nodes.as_str()
        ),
        ("v1.31.1+k3s1", "k3s", "3")
    );
}
