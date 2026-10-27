//! The restricted indicator (E17-S03): entries the user may not list are hidden, counted in a
//! "Limited access" line, and shown greyed with "Show all". Pure rules first, then the panel with
//! the real RBAC hook over a fake access port.

use std::collections::BTreeMap;

use gpui::TestAppContext;
use oxikube_app::kinds::AllowAll;
use oxikube_app::{AccessOutcome, KindGate, KindVisibility};
use oxikube_domain::access::AccessRules;
use oxikube_domain::ids::{ClusterId, Gvk, Scope};
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::discovery_sets;
use oxikube_ui::IconName;

use super::{Fixture, can_list, id};
use crate::sidebar::{
    AccessState, NoticeAction, Row, RowInputs, SidebarEntry, SidebarRegistry, SidebarSection,
    build_rows,
};

fn pod() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn secret() -> Gvk {
    Gvk::new("", "v1", "Secret")
}

/// One section with Pods and Secrets, each gated on its kind.
fn mixed() -> SidebarSection {
    let pods = SidebarEntry::kind("pods", "Pods", "", "pods").with_kind(pod(), Scope::Namespaced);
    let secrets = SidebarEntry::kind("secrets", "Secrets", "", "secrets")
        .with_kind(secret(), Scope::Namespaced);
    SidebarSection::new("k-mixed", "Mixed", IconName::Network, 500).with_entries([pods, secrets])
}

/// Hides Secrets everywhere.
struct NoSecrets;

impl KindVisibility for NoSecrets {
    fn is_listable(&self, _: &ClusterId, gvk: &Gvk, _: Option<&str>) -> Option<bool> {
        (&*gvk.kind == "Secret").then_some(false)
    }
}

fn rows_of(show_restricted: bool, visibility: &dyn KindVisibility) -> Vec<Row> {
    let cluster = id("prod");
    let namespaces = NamespaceSelection::All;
    let access = AccessState::Known(AccessOutcome::Reviewed(AccessRules::all_access()));
    let sections = [mixed()];
    build_rows(&RowInputs {
        sections: &sections,
        integrations: &[],
        custom: None,
        access: &access,
        crd_watch_forbidden: false,
        open: &BTreeMap::new(),
        custom_query: "",
        show_restricted,
        gate: Some(KindGate {
            cluster: &cluster,
            served: None,
            visibility,
            namespaces: &namespaces,
        }),
    })
}

fn notice(rows: &[Row]) -> Option<(String, Option<NoticeAction>)> {
    rows.iter().find_map(|row| match row {
        Row::Notice(n) if n.id == "access-limited" => Some((n.text.to_string(), n.action)),
        _ => None,
    })
}

fn entry(rows: &[Row], id: &str) -> Option<bool> {
    rows.iter().find_map(|row| match row {
        Row::Entry(e) if e.id == id => Some(e.restricted),
        _ => None,
    })
}

#[test]
fn a_denied_kind_is_hidden_and_the_indicator_offers_show_all() {
    let rows = rows_of(false, &NoSecrets);

    assert_eq!(entry(&rows, "k-mixed/pods"), Some(false));
    assert_eq!(entry(&rows, "k-mixed/secrets"), None, "hidden");
    assert_eq!(
        notice(&rows),
        Some((
            "Limited access: 1 kind hidden".to_owned(),
            Some(NoticeAction::ShowRestricted)
        ))
    );
}

#[test]
fn show_all_keeps_the_denied_kind_greyed_and_offers_to_hide_it() {
    let rows = rows_of(true, &NoSecrets);

    assert_eq!(entry(&rows, "k-mixed/pods"), Some(false));
    assert_eq!(entry(&rows, "k-mixed/secrets"), Some(true), "greyed");
    assert_eq!(
        notice(&rows),
        Some((
            "1 kind greyed out".to_owned(),
            Some(NoticeAction::HideRestricted)
        ))
    );
}

#[test]
fn a_section_of_only_denied_kinds_is_hidden_then_shown_greyed() {
    struct NothingListable;
    impl KindVisibility for NothingListable {
        fn is_listable(&self, _: &ClusterId, _: &Gvk, _: Option<&str>) -> Option<bool> {
            Some(false)
        }
    }
    let hidden = rows_of(false, &NothingListable);
    assert!(
        !hidden.iter().any(|r| r.id() == "k-mixed"),
        "the section goes when every kind is denied"
    );
    assert_eq!(
        notice(&hidden).map(|n| n.0),
        Some("Limited access: 1 section hidden".to_owned())
    );

    let shown = rows_of(true, &NothingListable);
    assert_eq!(entry(&shown, "k-mixed/pods"), Some(true));
    assert_eq!(entry(&shown, "k-mixed/secrets"), Some(true));
    assert!(
        shown
            .iter()
            .any(|r| matches!(r, Row::Section(s) if s.id == "k-mixed" && s.restricted)),
        "the heading is greyed too: {shown:?}"
    );
}

#[test]
fn nothing_denied_means_no_indicator() {
    let rows = rows_of(false, &AllowAll);
    assert_eq!(notice(&rows), None);
    assert_eq!(entry(&rows, "k-mixed/secrets"), Some(false));
}

#[gpui::test]
fn the_panel_hides_what_the_cached_rules_deny_and_show_all_greys_it(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_rbac(cx, can_list(&[("", "pods")]));
    fx.vcx
        .update(|_, cx| SidebarRegistry::register(cx, mixed()));
    fx.ports.discovery.set_kinds(discovery_sets::core());
    fx.select_namespace("dev");
    fx.connect_following();

    let rows = fx.rows();
    assert_eq!(entry(&rows, "k-mixed/pods"), Some(false), "{rows:?}");
    assert_eq!(entry(&rows, "k-mixed/secrets"), None, "{rows:?}");
    // The core sections this user may not list go too: the line counts them with the kind.
    let (text, action) = notice(&rows).expect("the indicator");
    assert!(text.ends_with("items hidden"), "{text}");
    assert_eq!(action, Some(NoticeAction::ShowRestricted));

    let panel = fx.panel.clone();
    fx.vcx
        .update(|_, cx| panel.update(cx, |p, cx| p.set_show_restricted(true, cx)));
    let rows = fx.rows();
    assert_eq!(entry(&rows, "k-mixed/secrets"), Some(true), "{rows:?}");
    assert!(fx.vcx.update(|_, cx| panel.read(cx).shows_restricted()));
}

#[gpui::test]
fn an_admin_sees_everything_and_no_indicator(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_rbac(cx, AccessRules::all_access());
    fx.vcx
        .update(|_, cx| SidebarRegistry::register(cx, mixed()));
    fx.ports.discovery.set_kinds(discovery_sets::core());
    fx.select_namespace("dev");
    fx.connect_following();

    let rows = fx.rows();
    assert_eq!(entry(&rows, "k-mixed/secrets"), Some(false), "{rows:?}");
    assert_eq!(notice(&rows), None);
}

#[gpui::test]
fn the_indicator_appears_when_the_namespace_changes_to_a_restricted_one(cx: &mut TestAppContext) {
    let mut fx = Fixture::open_rbac(cx, AccessRules::all_access());
    fx.ports
        .access
        .set_namespace_rules("locked", can_list(&[("", "pods")]));
    fx.vcx
        .update(|_, cx| SidebarRegistry::register(cx, mixed()));
    fx.ports.discovery.set_kinds(discovery_sets::core());
    fx.select_namespace("dev");
    fx.connect_following();
    assert_eq!(notice(&fx.rows()), None);

    fx.select_namespace("locked");

    let rows = fx.rows();
    assert_eq!(entry(&rows, "k-mixed/secrets"), None, "{rows:?}");
    assert!(notice(&rows).is_some(), "{rows:?}");
}
