use std::time::Duration;

use oxikube_app::portforward::ForwardFailure;
use oxikube_app::{ForwardId, ForwardInfo, ForwardState};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::portforward::{ForwardPort, ForwardSpec};
use oxikube_ports::{ForwardStatsSnapshot, PortForwardError};

use super::format::{bytes, traffic};
use super::{PanelModel, Row, StatusKind};

pub(crate) fn cluster(name: &str) -> ClusterId {
    ClusterId::new("kubeconfig", &ContextName::from(name))
}

/// A forward of `kind/name` in `ctx`'s `default` namespace, remote port 80, stopped.
pub(crate) fn info(id: u64, ctx: &str, kind: &str, name: &str) -> ForwardInfo {
    let target = ResourceRef::new(
        cluster(ctx),
        Gvk::new("", "v1", kind),
        Some("default".into()),
        name,
    );
    ForwardInfo {
        id: ForwardId(id),
        spec: ForwardSpec::new(target, ForwardPort::Number(80)),
        state: ForwardState::Stopped,
        local_addr: None,
        pod: None,
        last_error: None,
        stats: ForwardStatsSnapshot::default(),
        started_at: jiff::Timestamp::UNIX_EPOCH,
    }
}

pub(crate) fn active(mut info: ForwardInfo, port: u16) -> ForwardInfo {
    let addr = format!("127.0.0.1:{port}").parse().unwrap();
    info.state = ForwardState::Active {
        pod: "web-0".into(),
        local_addr: addr,
    };
    info.local_addr = Some(addr);
    info
}

/// The context name a test cluster was made from (the sessions would say so in the app).
pub(crate) fn label_of(cluster: &ClusterId) -> &'static str {
    ["0", "a", "b", "c", "dev", "prod"]
        .into_iter()
        .find(|name| cluster == &self::cluster(name))
        .unwrap_or("?")
}

fn row(info: &ForwardInfo) -> Row {
    let label = label_of(&info.spec.target.cluster);
    Row::new(info, label.into(), jiff::Timestamp::UNIX_EPOCH)
}

fn rows(infos: &[ForwardInfo]) -> Vec<Row> {
    infos.iter().map(row).collect()
}

fn ids(model: &PanelModel) -> Vec<u64> {
    model.rows().map(|r| r.id.0).collect()
}

#[test]
fn bytes_use_binary_units() {
    assert_eq!(bytes(0), "0 B");
    assert_eq!(bytes(1023), "1023 B");
    assert_eq!(bytes(1536), "1.5 KB");
    assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
    assert_eq!(bytes(3 * 1024 * 1024 * 1024), "3.0 GB");
    assert_eq!(bytes(u64::MAX), "16777216.0 TB");
}

#[test]
fn traffic_says_in_out_and_connections() {
    let stats = ForwardStatsSnapshot {
        bytes_in: 2048,
        bytes_out: 12,
        active_connections: 2,
        total_connections: 9,
    };
    assert_eq!(
        traffic(&stats),
        "\u{2193} 2.0 KB \u{2191} 12 B \u{b7} 2 conn"
    );
}

#[test]
fn an_active_row_shows_the_bound_port_target_and_uptime() {
    let mut i = active(info(1, "prod", "Pod", "web"), 18080);
    i.started_at = "2026-01-01T00:00:00Z".parse().unwrap();
    let now: jiff::Timestamp = "2026-01-01T00:05:00Z".parse().unwrap();
    let r = Row::new(&i, "prod".into(), now);
    assert_eq!(&*r.ports, "18080 -> 80");
    assert_eq!(&*r.target, "default/pod/web");
    assert_eq!(&*r.status, "Active \u{b7} web-0");
    assert_eq!(&*r.uptime, "5m");
    assert_eq!(r.url.as_deref(), Some("http://127.0.0.1:18080"));
    assert!(r.running);
    assert_eq!(r.kind, StatusKind::Active);
}

#[test]
fn an_unbound_automatic_port_says_auto() {
    let r = row(&info(1, "prod", "Service", "api"));
    assert_eq!(&*r.ports, "auto -> 80");
    assert_eq!(&*r.target, "default/service/api");
    assert_eq!(&*r.uptime, "-");
    assert_eq!(r.url, None);
    assert!(!r.running);
}

#[test]
fn a_reconnect_shows_the_attempt_and_the_next_retry() {
    let mut i = info(1, "prod", "Pod", "web");
    i.state = ForwardState::Reconnecting {
        attempt: 3,
        delay: Duration::from_millis(2400),
        reason: "the pod went away".into(),
    };
    let r = row(&i);
    assert_eq!(&*r.status, "Reconnecting \u{b7} attempt 3, next in 3s");
    assert_eq!(r.detail.as_deref(), Some("the pod went away"));
    assert!(r.running);

    i.state = ForwardState::Reconnecting {
        attempt: 1,
        delay: Duration::ZERO,
        reason: String::new(),
    };
    assert_eq!(
        &*row(&i).status,
        "Reconnecting \u{b7} attempt 1, retrying now"
    );
    i.state = ForwardState::Reconnecting {
        attempt: 5,
        delay: Duration::from_secs(90),
        reason: String::new(),
    };
    assert!(row(&i).status.contains("next in 1m 30s"));
}

#[test]
fn a_failed_row_carries_the_error_and_its_suggestion() {
    let mut i = info(1, "prod", "Pod", "web");
    i.state = ForwardState::Failed(ForwardFailure::from_error(PortForwardError::BindFailed {
        addr: "127.0.0.1:8080".parse().unwrap(),
        kind: std::io::ErrorKind::AddrInUse,
    }));
    let r = row(&i);
    assert_eq!(r.kind, StatusKind::Failed);
    assert!(!r.running);
    assert!(r.status.starts_with("Failed"));
    let detail = r.detail.expect("a failure explains itself");
    assert!(detail.contains("8080"), "{detail}");
    assert!(
        detail.contains("try another port"),
        "the suggestion is there: {detail}"
    );
}

#[test]
fn rows_are_ordered_by_cluster_then_target_then_id() {
    let mut model = PanelModel::new();
    model.set_rows(rows(&[
        info(4, "prod", "Pod", "web"),
        info(2, "dev", "Pod", "web"),
        info(3, "prod", "Pod", "api"),
        info(1, "dev", "Service", "db"),
    ]));
    // dev: default/pod/web (2), default/service/db (1); prod: default/pod/api (3), default/pod/web (4).
    assert_eq!(ids(&model), [2, 1, 3, 4]);
}

#[test]
fn an_unchanged_list_is_not_a_change() {
    let mut model = PanelModel::new();
    let list = rows(&[info(1, "dev", "Pod", "web")]);
    assert!(model.set_rows(list.clone()));
    assert!(!model.set_rows(list));
}

#[test]
fn the_text_filter_looks_in_cluster_target_ports_and_status() {
    let mut model = PanelModel::new();
    model.set_rows(rows(&[
        active(info(1, "dev", "Pod", "web"), 18080),
        info(2, "prod", "Service", "api"),
    ]));
    assert!(model.set_text("API"));
    assert_eq!(ids(&model), [2]);
    model.set_text("active");
    assert_eq!(ids(&model), [1]);
    model.set_text("18080");
    assert_eq!(ids(&model), [1]);
    model.set_text("");
    assert_eq!(ids(&model), [1, 2]);
    assert!(!model.set_text(""));
}

#[test]
fn the_cluster_filter_cycles_through_clusters_and_back_to_all() {
    let mut model = PanelModel::new();
    model.set_rows(rows(&[
        info(1, "dev", "Pod", "web"),
        info(2, "prod", "Pod", "web"),
    ]));
    assert_eq!(model.clusters().len(), 2);
    model.cycle_cluster();
    assert_eq!(ids(&model), [1]);
    model.cycle_cluster();
    assert_eq!(ids(&model), [2]);
    model.cycle_cluster();
    assert_eq!(ids(&model), [1, 2]);
    assert_eq!(model.cluster_filter(), None);
}

#[test]
fn the_selection_follows_the_forward_when_rows_shift() {
    let mut model = PanelModel::new();
    model.set_rows(rows(&[
        info(1, "a", "Pod", "x"),
        info(2, "b", "Pod", "x"),
        info(3, "c", "Pod", "x"),
    ]));
    model.select(1);
    assert_eq!(model.selected().map(|r| r.id.0), Some(2));
    // A forward appears above it: the selection stays on forward 2.
    model.set_rows(rows(&[
        info(0, "0", "Pod", "x"),
        info(1, "a", "Pod", "x"),
        info(2, "b", "Pod", "x"),
        info(3, "c", "Pod", "x"),
    ]));
    assert_eq!(model.selected().map(|r| r.id.0), Some(2));
    assert_eq!(model.selected_ix(), Some(2));
}

#[test]
fn removing_the_selected_forward_selects_its_neighbour() {
    let mut model = PanelModel::new();
    model.set_rows(rows(&[
        info(1, "a", "Pod", "x"),
        info(2, "b", "Pod", "x"),
        info(3, "c", "Pod", "x"),
    ]));
    model.select(1);
    model.set_rows(rows(&[info(1, "a", "Pod", "x"), info(3, "c", "Pod", "x")]));
    assert_eq!(model.selected().map(|r| r.id.0), Some(3));
    model.set_rows(rows(&[info(1, "a", "Pod", "x")]));
    assert_eq!(model.selected().map(|r| r.id.0), Some(1));
    model.set_rows(Vec::new());
    assert!(model.selected().is_none());
}

#[test]
fn stepping_clamps_and_the_ends_report_a_move_only_when_they_move() {
    let mut model = PanelModel::new();
    assert_eq!(model.step(1), None, "nothing to select");
    model.set_rows(rows(&[info(1, "a", "Pod", "x"), info(2, "b", "Pod", "x")]));
    assert_eq!(
        model.step(1),
        Some(0),
        "the first step selects the first line"
    );
    assert_eq!(model.step(1), Some(1));
    assert_eq!(model.step(1), None, "clamped at the end");
    assert_eq!(model.select_end(false), Some(0));
    assert_eq!(model.select_end(false), None);
    assert_eq!(model.select_end(true), Some(1));
}

#[test]
fn counts_include_the_filtered_out() {
    let mut model = PanelModel::new();
    let mut failed = info(2, "b", "Pod", "x");
    failed.state = ForwardState::Failed(ForwardFailure::message("nope"));
    model.set_rows(rows(&[info(1, "a", "Pod", "x"), failed]));
    model.set_text("a/");
    assert_eq!((model.len(), model.total(), model.failed()), (0, 2, 1));
}
