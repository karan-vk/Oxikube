//! The panel over testkit fakes: reviews after connect, namespace changes, reconnects, fail-open,
//! saved state, navigation and keys.

use std::{cell::RefCell, rc::Rc};

use gpui::{Bounds, Focusable as _, Pixels, Point, TestAppContext, point};
use oxikube_domain::Capabilities;
use oxikube_domain::access::AccessRules;
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::{SidebarItem, SidebarModel, SidebarSection};
use oxikube_testkit::{AccessCall, FakeIntegrationPort};

use super::{Fixture, can_list, crd};
use crate::sidebar::{NoticeKind, Row, SidebarEvent, SidebarTarget};

fn center(bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    )
}

fn notice(rows: &[Row], id: &str) -> Option<(NoticeKind, String)> {
    rows.iter().find_map(|row| match row {
        Row::Notice(n) if n.id == id => Some((n.kind, n.text.to_string())),
        _ => None,
    })
}

fn draw(fx: &mut Fixture) {
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
}

fn click(fx: &mut Fixture, selector: &'static str) {
    draw(fx);
    let bounds = fx
        .vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is on screen"));
    fx.vcx.simulate_click(center(bounds), Default::default());
    fx.vcx.run_until_parked();
}

#[gpui::test]
fn before_connecting_only_the_overview_shows(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    assert!(fx.dock_open());
    assert_eq!(fx.sections(), ["cluster"]);
    assert_eq!(
        notice(&fx.rows(), "access-pending").map(|n| n.0),
        Some(NoticeKind::Muted)
    );
}

#[gpui::test]
fn ready_is_not_delayed_by_the_review_and_visibility_follows_it(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, can_list(&[("", "pods"), ("", "services")]));
    // The connect future resolves (Ready) without the sidebar's review having run.
    futures::executor::block_on(fx.sessions.connect(&fx.cluster)).expect("connect");
    assert!(fx.sessions.get(&fx.cluster).unwrap().is_connected());
    assert_eq!(
        fx.ports.access.recorded_calls(),
        [AccessCall::Capabilities(None)],
        "only the connect probe so far"
    );
    assert_eq!(fx.sections(), ["cluster"]);

    // Then the sidebar reviews, and the restricted user's sections appear.
    fx.vcx.run_until_parked();
    assert_eq!(fx.sections(), ["cluster", "workloads", "network"]);
    let (kind, text) = notice(&fx.rows(), "access-limited").expect("limited access hint");
    assert_eq!(kind, NoticeKind::Muted);
    assert!(text.contains("hidden"), "{text}");
}

#[gpui::test]
fn a_full_admin_sees_every_section_once_custom_resources_exist(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect();
    // No CRD on the cluster: Custom Resources is hidden entirely.
    assert_eq!(fx.sections().len(), 10);
    assert!(!fx.sections().contains(&"custom-resources".to_owned()));
    assert!(notice(&fx.rows(), "access-limited").is_none());

    // A cluster with CRDs shows it (discovery runs again on the next connect).
    fx.ports
        .discovery
        .set_kinds([crd("argoproj.io", "Application", "applications")]);
    fx.disconnect();
    fx.connect();
    assert_eq!(fx.sections().len(), 11);
    assert_eq!(
        fx.sections().last().map(String::as_str),
        Some("custom-resources")
    );
}

#[gpui::test]
fn a_failed_review_shows_everything_with_a_warning(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, can_list(&[("", "pods")]));
    fx.ports
        .access
        .script()
        .rules
        .push_err(OxiError::new(ErrorKind::Network, "connection reset"));
    fx.connect();
    assert_eq!(fx.sections().len(), 10, "nothing is hidden by mistake");
    let rows = fx.rows();
    let Some(Row::Notice(warning)) = rows.first() else {
        panic!("the warning leads: {rows:?}");
    };
    assert_eq!(warning.kind, NoticeKind::Warning);
    assert!(warning.text.contains("connection reset"));
}

#[gpui::test]
fn changing_the_namespace_selection_recomputes_visibility(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::none());
    // `dev` may list deployments, `ops` may list secrets; nothing cluster-wide.
    fx.ports
        .access
        .set_namespace_rules("dev", can_list(&[("apps", "deployments")]));
    fx.ports
        .access
        .set_namespace_rules("ops", can_list(&[("", "secrets")]));
    fx.connect();
    assert_eq!(
        fx.sections(),
        ["cluster"],
        "All namespaces: nothing granted"
    );

    let select = |fx: &mut Fixture, names: &[&str]| {
        fx.sessions
            .set_namespace_selection(&fx.cluster, NamespaceSelection::from_names(names))
            .expect("open session");
        fx.vcx.run_until_parked();
    };
    select(&mut fx, &["dev"]);
    assert_eq!(fx.sections(), ["cluster", "workloads"]);
    assert!(
        fx.ports
            .access
            .recorded_calls()
            .contains(&AccessCall::Rules(Some("dev".into())))
    );

    select(&mut fx, &["ops"]);
    assert_eq!(fx.sections(), ["cluster", "config", "helm"]);

    // Both selected: the union.
    select(&mut fx, &["dev", "ops"]);
    assert_eq!(fx.sections(), ["cluster", "workloads", "config", "helm"]);

    // Back to all namespaces.
    select(&mut fx, &[]);
    assert_eq!(fx.sections(), ["cluster"]);
}

#[gpui::test]
fn a_reconnect_reviews_again(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, can_list(&[("", "pods")]));
    fx.connect();
    assert_eq!(fx.sections(), ["cluster", "workloads"]);

    // The administrator grants more while the user is away.
    fx.ports
        .access
        .set_rules(can_list(&[("", "pods"), ("", "nodes")]));
    fx.disconnect();
    fx.connect();
    assert_eq!(fx.sections(), ["cluster", "nodes", "workloads"]);
}

#[gpui::test]
fn a_health_flap_is_not_a_reconnect(cx: &mut TestAppContext) {
    use oxikube_ports::HealthSignal;

    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect();
    let reviews = |fx: &Fixture| {
        fx.ports
            .access
            .recorded_calls()
            .iter()
            .filter(|c| matches!(c, AccessCall::Rules(_)))
            .count()
    };
    assert_eq!(reviews(&fx), 1);
    fx.sessions
        .report_health(&fx.cluster, HealthSignal::Unhealthy);
    fx.vcx.run_until_parked();
    fx.sessions
        .report_health(&fx.cluster, HealthSignal::Healthy);
    fx.vcx.run_until_parked();
    assert_eq!(
        reviews(&fx),
        1,
        "Ready -> Degraded -> Ready is one connection"
    );
}

#[gpui::test]
fn integration_sections_registered_later_come_after_the_core_ones(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.integrations
        .register(std::sync::Arc::new(
            FakeIntegrationPort::new("argocd").with_sidebar(SidebarModel::single(SidebarSection {
                id: "apps".into(),
                title: "Applications".into(),
                icon: None,
                items: vec![SidebarItem {
                    id: "list".into(),
                    title: "Applications".into(),
                    icon: None,
                    command: oxikube_domain::command::CommandId::new("argo::Open"),
                    needs: Capabilities::ARGO,
                }],
            })),
        ))
        .unwrap();
    fx.connect();
    let sections = fx.sections();
    assert_eq!(
        sections.last().map(String::as_str),
        Some("integration:argocd/apps")
    );
    assert_eq!(
        sections.len(),
        11,
        "ten core sections and the integration's"
    );
    assert!(
        sections[..10]
            .iter()
            .all(|s| !s.starts_with("integration:")),
        "core first: {sections:?}"
    );
}

#[gpui::test]
fn collapsed_state_is_saved_per_cluster_and_restored(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.ports
        .discovery
        .set_kinds([crd("argoproj.io", "Application", "applications")]);
    fx.connect();
    assert!(fx.is_open("workloads"));
    assert!(fx.row_ids().contains(&"workloads/pods".to_owned()));
    // The custom-resource group starts closed.
    assert!(!fx.is_open("crd:argoproj.io"));

    assert!(fx.toggle("workloads"), "collapse");
    assert!(fx.toggle("crd:argoproj.io"), "expand");
    assert!(!fx.is_open("workloads"));
    assert!(
        !fx.row_ids().contains(&"workloads/pods".to_owned()),
        "its entries are gone"
    );
    assert!(
        fx.row_ids()
            .contains(&"crd:argoproj.io/applications".to_owned())
    );
    assert!(!fx.toggle("workloads/pods"), "an entry is not a group");

    // A new window over the same state (the next launch): same choices, once connected.
    let state = fx.state.clone();
    drop(fx);
    let mut again = Fixture::open_with(cx, AccessRules::all_access(), state);
    again
        .ports
        .discovery
        .set_kinds([crd("argoproj.io", "Application", "applications")]);
    again.connect();
    assert!(!again.is_open("workloads"));
    assert!(again.is_open("crd:argoproj.io"));
    assert!(
        again.is_open("network"),
        "what was not touched keeps its default"
    );

    // Expanding again persists too.
    assert!(again.toggle("workloads"));
    let state = again.state.clone();
    drop(again);
    let mut third = Fixture::open_with(cx, AccessRules::all_access(), state);
    third.connect();
    assert!(third.is_open("workloads"));
}

#[gpui::test]
fn another_clusters_state_is_not_touched(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect();
    fx.toggle("workloads");
    // The state is keyed by cluster id: a different cluster's sidebar starts from defaults.
    let store = crate::sidebar::SidebarStore::new(fx.state.clone(), &super::id("staging"))
        .expect("a valid key");
    assert!(futures::executor::block_on(store.load()).unwrap().is_none());
    let mine = crate::sidebar::SidebarStore::new(fx.state.clone(), &fx.cluster).unwrap();
    let saved = futures::executor::block_on(mine.load())
        .unwrap()
        .expect("saved");
    assert_eq!(saved.open.get("workloads"), Some(&false));
}

#[gpui::test]
fn clicking_a_heading_opens_and_closes_it(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect();
    assert!(fx.is_open("config"));
    click(&mut fx, "sidebar-section-config");
    assert!(!fx.is_open("config"));
    click(&mut fx, "sidebar-section-config");
    assert!(fx.is_open("config"));
    // The count placeholder is drawn on each heading.
    draw(&mut fx);
    assert!(fx.vcx.debug_bounds("sidebar-count-config").is_some());
}

#[gpui::test]
fn activating_an_entry_navigates_and_a_link_heading_too(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect();
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let panel = fx.panel.clone();
    let _subscription = fx.vcx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &SidebarEvent, _| {
            sink.borrow_mut().push(event.clone());
        })
    });

    click(&mut fx, "sidebar-entry-workloads/pods");
    click(&mut fx, "sidebar-section-cluster");
    assert_eq!(
        *events.borrow(),
        [
            SidebarEvent::Navigate(SidebarTarget::kind("", "pods")),
            SidebarEvent::Navigate(SidebarTarget::Page("overview".into())),
        ]
    );
    let panel = fx.panel.clone();
    let selected = fx
        .vcx
        .update(|_, cx| panel.read(cx).selected().map(str::to_owned));
    assert_eq!(selected.as_deref(), Some("cluster"));
}

#[gpui::test]
fn the_keyboard_walks_the_list(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect();
    let panel = fx.panel.clone();
    fx.vcx.update(|window, cx| {
        let focus = panel.focus_handle(cx);
        window.focus(&focus, cx);
    });
    draw(&mut fx);

    fx.vcx.simulate_keystrokes("down");
    let highlighted = |fx: &mut Fixture| {
        let panel = fx.panel.clone();
        fx.vcx
            .update(|_, cx| panel.read(cx).highlighted().map(str::to_owned))
    };
    assert_eq!(highlighted(&mut fx).as_deref(), Some("cluster"));
    fx.vcx.simulate_keystrokes("down");
    assert_eq!(highlighted(&mut fx).as_deref(), Some("nodes"));

    // Left closes the highlighted section, right opens it, enter flips it.
    assert!(fx.is_open("nodes"));
    fx.vcx.simulate_keystrokes("left");
    assert!(!fx.is_open("nodes"));
    fx.vcx.simulate_keystrokes("right");
    assert!(fx.is_open("nodes"));
    fx.vcx.simulate_keystrokes("enter");
    assert!(!fx.is_open("nodes"));
    fx.vcx.simulate_keystrokes("up");
    assert_eq!(highlighted(&mut fx).as_deref(), Some("cluster"));
}
