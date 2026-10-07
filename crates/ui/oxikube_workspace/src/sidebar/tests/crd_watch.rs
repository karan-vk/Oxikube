//! The sidebar follows CRDs added and removed while connected, and says so when the user may not
//! watch them (E03-F544). The session starts the adapter's watch on connect; these tests play the
//! adapter through the fake discovery port.

use gpui::TestAppContext;
use oxikube_domain::access::AccessRules;
use oxikube_domain::ids::Gvk;
use oxikube_ports::{CrdWatchStatus, DiscoveryEvent, KindsChange};

use super::{Fixture, crd};
use crate::sidebar::{NoticeKind, Row};

fn change(added: &[&str], removed: &[&str]) -> DiscoveryEvent {
    let gvks = |kinds: &[&str]| {
        kinds
            .iter()
            .map(|k| Gvk::new("example.io", "v1", *k))
            .collect()
    };
    DiscoveryEvent::KindsChanged(KindsChange {
        added: gvks(added),
        removed: gvks(removed),
        changed: Vec::new(),
    })
}

fn has_notice(rows: &[Row], id: &str) -> Option<NoticeKind> {
    rows.iter().find_map(|row| match row {
        Row::Notice(n) if n.id == id => Some(n.kind),
        _ => None,
    })
}

#[gpui::test]
fn a_crd_added_while_connected_appears_and_its_removal_disappears(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect_following();
    assert!(!fx.sections().contains(&"custom-resources".to_owned()));
    assert_eq!(
        fx.ports.discovery.live_subscriptions(),
        1,
        "the session started the watch"
    );

    // A CRD is created: discovery serves it and the adapter reports the change.
    fx.ports
        .discovery
        .set_kinds([crd("example.io", "Widget", "widgets")]);
    fx.emit(change(&["Widget"], &[]));
    assert!(fx.sections().contains(&"custom-resources".to_owned()));
    assert!(
        fx.row_ids().contains(&"crd:example.io".to_owned()),
        "{:?}",
        fx.row_ids()
    );

    // Deleted again.
    fx.ports.discovery.set_kinds([]);
    fx.emit(change(&[], &["Widget"]));
    assert!(!fx.sections().contains(&"custom-resources".to_owned()));
}

#[gpui::test]
fn a_refused_crd_watch_is_a_visible_notice_until_it_recovers(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect_following();
    assert_eq!(has_notice(&fx.rows(), "crd-watch-forbidden"), None);

    fx.emit(DiscoveryEvent::CrdWatch(CrdWatchStatus::Forbidden {
        reason: "customresourcedefinitions is forbidden".into(),
    }));
    assert_eq!(
        has_notice(&fx.rows(), "crd-watch-forbidden"),
        Some(NoticeKind::Muted)
    );

    fx.emit(DiscoveryEvent::CrdWatch(CrdWatchStatus::Watching));
    assert_eq!(has_notice(&fx.rows(), "crd-watch-forbidden"), None);
}

#[gpui::test]
fn the_notice_is_dropped_when_the_connection_goes_away(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect_following();
    fx.emit(DiscoveryEvent::CrdWatch(CrdWatchStatus::Forbidden {
        reason: "forbidden".into(),
    }));
    assert!(has_notice(&fx.rows(), "crd-watch-forbidden").is_some());

    fx.disconnect();
    assert_eq!(has_notice(&fx.rows(), "crd-watch-forbidden"), None);
}
