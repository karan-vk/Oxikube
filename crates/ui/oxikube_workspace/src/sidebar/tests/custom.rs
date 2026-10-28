//! Custom Resources (E12-S17): grouping by API group, kinds another section claims left out, the
//! search field (pure rows, then typed into the panel's input), and the filter's cost with 500
//! CRDs.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

use gpui::TestAppContext;
use oxikube_app::{AccessOutcome, CustomKind, CustomResourceGroup};
use oxikube_domain::access::AccessRules;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_ui::IconName;

use super::{Fixture, crd};
use crate::sidebar::{
    AccessState, Row, RowInputs, SEARCH_ROW, SidebarEntry, SidebarEvent, SidebarRegistry,
    SidebarSection, SidebarTarget, build_rows, core_sections,
};

fn group(name: &str, kinds: &[(&str, &str)]) -> CustomResourceGroup {
    CustomResourceGroup {
        group: name.to_owned(),
        kinds: kinds
            .iter()
            .map(|(kind, plural)| CustomKind {
                kind: (*kind).to_owned(),
                plural: (*plural).to_owned(),
                version: "v1".to_owned(),
                namespaced: true,
            })
            .collect(),
    }
}

fn found() -> Vec<CustomResourceGroup> {
    vec![
        group(
            "argoproj.io",
            &[
                ("AppProject", "appprojects"),
                ("Application", "applications"),
            ],
        ),
        group(
            "cert-manager.io",
            &[("Certificate", "certificates"), ("Issuer", "issuers")],
        ),
        group(
            "gateway.networking.k8s.io",
            &[("Gateway", "gateways"), ("HTTPRoute", "httproutes")],
        ),
        group(
            "snapshot.storage.k8s.io",
            &[("VolumeSnapshot", "volumesnapshots")],
        ),
    ]
}

/// The core sections, with Network listing the Gateway API and Storage VolumeSnapshots as the
/// kind registry publishes them (E12-S14, E12-S15).
fn sections() -> Vec<SidebarSection> {
    let mut sections = core_sections();
    sections.push(
        SidebarSection::new("network-kinds", "Network", IconName::Plug, 450).with_entries([
            SidebarEntry::kind(
                "gateways",
                "Gateways",
                "gateway.networking.k8s.io",
                "gateways",
            )
            .with_kind(
                Gvk::new("gateway.networking.k8s.io", "v1", "Gateway"),
                Scope::Namespaced,
            ),
            SidebarEntry::kind(
                "httproutes",
                "HTTP Routes",
                "gateway.networking.k8s.io",
                "httproutes",
            )
            .with_kind(
                Gvk::new("gateway.networking.k8s.io", "v1", "HTTPRoute"),
                Scope::Namespaced,
            ),
            SidebarEntry::kind(
                "snapshots",
                "Volume Snapshots",
                "snapshot.storage.k8s.io",
                "volumesnapshots",
            )
            .with_kind(
                Gvk::new("snapshot.storage.k8s.io", "v1", "VolumeSnapshot"),
                Scope::Namespaced,
            ),
        ]),
    );
    sections
}

fn rows(found: &[CustomResourceGroup], open: &BTreeMap<String, bool>, query: &str) -> Vec<Row> {
    build_rows(&RowInputs {
        sections: &sections(),
        integrations: &[],
        custom: Some(found),
        access: &AccessState::Known(AccessOutcome::Reviewed(AccessRules::all_access())),
        crd_watch_forbidden: false,
        open,
        custom_query: query,
        show_restricted: false,
        gate: None,
    })
}

/// The ids of the Custom Resources section's rows, its heading first.
fn custom_ids(rows: &[Row]) -> Vec<String> {
    let at = rows
        .iter()
        .position(|r| r.id() == "custom-resources")
        .expect("the section");
    let end = rows[at + 1..]
        .iter()
        .position(|r| matches!(r, Row::Section(_)))
        .map_or(rows.len(), |n| at + 1 + n);
    rows[at..end].iter().map(|r| r.id().to_owned()).collect()
}

#[test]
fn groups_are_by_api_group_and_kinds_other_sections_claim_are_left_out() {
    let ids = custom_ids(&rows(&found(), &BTreeMap::new(), ""));
    assert_eq!(
        ids,
        [
            "custom-resources",
            "custom-resources/definitions",
            SEARCH_ROW,
            "crd:argoproj.io",
            "crd:cert-manager.io",
        ],
        "the Gateway API and VolumeSnapshots are listed by Network and Storage, not twice"
    );
    // Every kind of a group is claimed except one: that one stays.
    let mut more = found();
    more[2].kinds.push(CustomKind {
        kind: "BackendTLSPolicy".into(),
        plural: "backendtlspolicies".into(),
        version: "v1alpha3".into(),
        namespaced: true,
    });
    let open = BTreeMap::from([("crd:gateway.networking.k8s.io".to_owned(), true)]);
    let ids = custom_ids(&rows(&more, &open, ""));
    assert!(ids.contains(&"crd:gateway.networking.k8s.io/backendtlspolicies".to_owned()));
    assert!(!ids.contains(&"crd:gateway.networking.k8s.io/gateways".to_owned()));
    // Only claimed kinds: the section still leads to the definitions.
    let ids = custom_ids(&rows(&found()[2..], &BTreeMap::new(), ""));
    assert_eq!(ids, ["custom-resources", "custom-resources/definitions"]);
}

#[test]
fn the_search_shows_matching_groups_open_and_says_when_nothing_matches() {
    let rows_for = |query: &str| custom_ids(&rows(&found(), &BTreeMap::new(), query));
    // A kind's name: its group, opened, with only that kind.
    assert_eq!(
        rows_for("cert"),
        [
            "custom-resources",
            "custom-resources/definitions",
            SEARCH_ROW,
            "crd:cert-manager.io",
            "crd:cert-manager.io/certificates",
            "crd:cert-manager.io/issuers",
        ],
        "the group's name matches: all its kinds"
    );
    assert_eq!(
        rows_for("ISSUER")[3..],
        ["crd:cert-manager.io", "crd:cert-manager.io/issuers"]
    );
    let counted = rows(&found(), &BTreeMap::new(), "app");
    let count = counted.iter().find_map(|r| match r {
        Row::Group(g) if &*g.id == "crd:argoproj.io" => g.count,
        _ => None,
    });
    assert_eq!(count, Some(2), "AppProject and Application match");
    // Claimed kinds never come back through the search.
    assert!(
        rows_for("gateway")
            .last()
            .is_some_and(|id| id == "custom-resources-no-match")
    );
    let search = rows(&found(), &BTreeMap::new(), "  cert ");
    assert!(
        search
            .iter()
            .any(|r| matches!(r, Row::Search(s) if &*s.query == "cert"))
    );
    // A closed group the user chose stays closed without a query and opens while it matches.
    let closed = BTreeMap::from([("crd:argoproj.io".to_owned(), false)]);
    assert!(
        !custom_ids(&rows(&found(), &closed, ""))
            .contains(&"crd:argoproj.io/applications".to_owned())
    );
    assert!(
        custom_ids(&rows(&found(), &closed, "argo"))
            .contains(&"crd:argoproj.io/applications".to_owned())
    );
}

#[test]
fn filtering_500_crds_takes_well_under_a_frame() {
    // 100 groups of 5 kinds: the size the story's budget names.
    let many: Vec<CustomResourceGroup> = (0..100)
        .map(|g| {
            let kinds: Vec<(String, String)> = (0..5)
                .map(|k| (format!("Kind{g}x{k}"), format!("kind{g}x{k}s")))
                .collect();
            let refs: Vec<(&str, &str)> = kinds
                .iter()
                .map(|(a, b)| (a.as_str(), b.as_str()))
                .collect();
            group(&format!("group{g}.example.com"), &refs)
        })
        .collect();
    let open = BTreeMap::new();
    let queries = [
        "k", "ki", "kin", "kind", "kind4", "kind42", "kind42x", "kind42x3", "group9", "",
    ];
    let rounds = 20;
    let started = Instant::now();
    for _ in 0..rounds {
        for query in queries {
            std::hint::black_box(rows(&many, &open, query));
        }
    }
    let per_keystroke = started.elapsed() / (rounds * queries.len() as u32);
    eprintln!("sidebar filter over 500 CRDs: {per_keystroke:?} per keystroke (rows rebuilt)");
    // A debug build on a loaded CI runner stays far below a frame (8 ms); release is ~10x less.
    assert!(per_keystroke.as_millis() < 8, "{per_keystroke:?}");
    let all = rows(&many, &open, "kind");
    assert_eq!(
        all.iter()
            .filter(|r| matches!(r, Row::Entry(e) if e.depth == 2))
            .count(),
        500
    );
}

/// Opens a cluster with three custom kinds, closes the long sections so the search row is on
/// screen, and moves the keyboard focus into the search field the way Enter on its row does.
fn focused_search(cx: &mut TestAppContext) -> Fixture {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.ports.discovery.set_kinds([
        crd("argoproj.io", "Application", "applications"),
        crd("argoproj.io", "Rollout", "rollouts"),
        crd("cert-manager.io", "Certificate", "certificates"),
    ]);
    fx.connect();
    for id in [
        "workloads",
        "config",
        "network",
        "storage",
        "access-control",
    ] {
        fx.toggle(id);
    }
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        fx.vcx.debug_bounds("sidebar-custom-search").is_some(),
        "the field is drawn"
    );
    let panel = fx.panel.clone();
    let input = fx
        .vcx
        .update(|_, cx| panel.read(cx).search_field().expect("built on draw"));
    fx.vcx
        .update(|_, cx| panel.update(cx, |p, cx| p.activate(SEARCH_ROW, cx)));
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let focused = fx
        .vcx
        .update(|window, cx| gpui::Focusable::focus_handle(input.read(cx), cx).is_focused(window));
    assert!(focused, "the field has the keyboard focus");
    fx
}

/// Records the panel's events.
fn sidebar_events(fx: &mut Fixture) -> (Rc<RefCell<Vec<SidebarEvent>>>, gpui::Subscription) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sink = events.clone();
    let panel = fx.panel.clone();
    let subscription = fx.vcx.update(|_, cx| {
        cx.subscribe(&panel, move |_, event: &SidebarEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
    });
    (events, subscription)
}

#[gpui::test]
fn typing_in_the_search_filters_and_enter_opens_the_first_match(cx: &mut TestAppContext) {
    let mut fx = focused_search(cx);
    let panel = fx.panel.clone();
    let (events, _subscription) = sidebar_events(&mut fx);

    // The field has the focus (Enter on its row put it there); the rows follow each keystroke.
    fx.vcx.simulate_input("roll");
    fx.vcx.run_until_parked();
    let query = fx
        .vcx
        .update(|_, cx| panel.read(cx).custom_query().to_owned());
    assert_eq!(query, "roll");
    let ids = fx.row_ids();
    assert!(
        ids.contains(&"crd:argoproj.io/rollouts".to_owned()),
        "{ids:?}"
    );
    assert!(!ids.contains(&"crd:argoproj.io/applications".to_owned()));
    assert!(!ids.contains(&"crd:cert-manager.io".to_owned()));

    fx.vcx.simulate_keystrokes("enter");
    fx.vcx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        [SidebarEvent::Navigate(SidebarTarget::kind(
            "argoproj.io",
            "rollouts"
        ))]
    );

    // Clearing the query brings the saved (closed) state back; nothing about it was saved.
    fx.vcx
        .update(|_, cx| panel.update(cx, |p, cx| p.set_custom_query("", cx)));
    assert!(
        !fx.row_ids()
            .contains(&"crd:argoproj.io/rollouts".to_owned())
    );
    assert!(!fx.is_open("crd:argoproj.io"));
}

/// The sidebar binds space to Activate; in the search field a space is text (reviewer finding on
/// PR #656), so it neither opens the highlighted row nor gets lost.
#[gpui::test]
fn a_space_typed_in_the_search_is_text_and_activates_nothing(cx: &mut TestAppContext) {
    let mut fx = focused_search(cx);
    let panel = fx.panel.clone();
    let (events, _subscription) = sidebar_events(&mut fx);

    fx.vcx.simulate_input("argo");
    let space = gpui::Keystroke::parse("space-> ").expect("a keystroke");
    fx.vcx
        .update(|window, cx| window.dispatch_keystroke(space, cx));
    fx.vcx.simulate_input("proj");
    fx.vcx.run_until_parked();

    let query = fx
        .vcx
        .update(|_, cx| panel.read(cx).custom_query().to_owned());
    assert_eq!(query, "argo proj", "the space reached the field");
    assert!(events.borrow().is_empty(), "{:?}", events.borrow());
    assert!(
        !fx.row_ids()
            .contains(&"crd:argoproj.io/rollouts".to_owned()),
        "nothing matches the spaced query, so no kind shows"
    );
}

#[gpui::test]
fn a_kind_registered_in_another_section_is_not_a_custom_resource_too(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.ports.discovery.set_kinds([
        crd("gateway.networking.k8s.io", "Gateway", "gateways"),
        crd("argoproj.io", "Application", "applications"),
    ]);
    fx.vcx.update(|_, cx| {
        SidebarRegistry::add_entry(
            cx,
            "network",
            SidebarEntry::kind(
                "gateways",
                "Gateways",
                "gateway.networking.k8s.io",
                "gateways",
            )
            .with_kind(
                Gvk::new("gateway.networking.k8s.io", "v1", "Gateway"),
                Scope::Namespaced,
            ),
        );
    });
    fx.connect();
    let ids = fx.row_ids();
    assert!(ids.contains(&"network/gateways".to_owned()), "{ids:?}");
    assert!(
        !ids.contains(&"crd:gateway.networking.k8s.io".to_owned()),
        "{ids:?}"
    );
    assert!(ids.contains(&"crd:argoproj.io".to_owned()));
}
