//! The registry: sections come from registration, in order, and open sidebars follow it.

use gpui::TestAppContext;
use oxikube_domain::access::{AccessRequirement, AccessRules};
use oxikube_ui::IconName;

use super::Fixture;
use crate::sidebar::{SidebarEntry, SidebarRegistry, SidebarSection, SidebarTarget, core_sections};

#[gpui::test]
fn core_sections_are_registered_by_init_in_order(cx: &mut TestAppContext) {
    cx.update(crate::sidebar::init);
    let ids: Vec<_> = cx.update(|cx| {
        SidebarRegistry::sections(cx)
            .into_iter()
            .map(|s| s.id.to_string())
            .collect()
    });
    let expected: Vec<_> = core_sections()
        .into_iter()
        .map(|s| s.id.to_string())
        .collect();
    assert_eq!(ids, expected);
}

#[gpui::test]
fn registering_twice_replaces_and_orders_by_order_then_registration(cx: &mut TestAppContext) {
    cx.update(|cx| {
        SidebarRegistry::register(cx, SidebarSection::new("b", "B", IconName::Box, 20));
        SidebarRegistry::register(cx, SidebarSection::new("a", "A", IconName::Box, 20));
        SidebarRegistry::register(cx, SidebarSection::new("first", "First", IconName::Box, 1));
        SidebarRegistry::register(cx, SidebarSection::new("b", "B again", IconName::Box, 20));
    });
    let sections: Vec<_> = cx.update(|cx| {
        SidebarRegistry::sections(cx)
            .into_iter()
            .map(|s| (s.id.to_string(), s.title.to_string()))
            .collect()
    });
    assert_eq!(
        sections,
        [
            ("first".into(), "First".into()),
            ("b".into(), "B again".into()),
            ("a".into(), "A".into()),
        ]
    );
}

#[gpui::test]
fn a_kind_adds_an_entry_to_a_core_section_and_open_sidebars_redraw(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect();
    assert!(!fx.row_ids().contains(&"workloads/gateways".to_owned()));

    let added = fx.vcx.update(|_, cx| {
        SidebarRegistry::add_entry(
            cx,
            "workloads",
            SidebarEntry::kind("gateways", "Gateways", "gateway.example.io", "gateways"),
        )
    });
    fx.vcx.run_until_parked();
    // The admin may list it, so it shows; a user who may not would not see it.
    assert!(added);
    assert!(fx.row_ids().contains(&"workloads/gateways".to_owned()));
    let none = fx.vcx.update(|_, cx| {
        SidebarRegistry::add_entry(cx, "no-such-section", SidebarEntry::kind("x", "X", "", "x"))
    });
    assert!(!none);
}

#[gpui::test]
fn a_section_registered_after_the_sidebar_opened_appears(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, AccessRules::all_access());
    fx.connect();
    fx.vcx.update(|_, cx| {
        SidebarRegistry::register(
            cx,
            SidebarSection::new("flux", "Flux", IconName::Workflow, 1_500)
                .with_requires([AccessRequirement::list(
                    "kustomize.toolkit.fluxcd.io",
                    "kustomizations",
                )])
                .with_target(SidebarTarget::Page("flux".into())),
        )
    });
    fx.vcx.run_until_parked();
    let sections = fx.sections();
    assert_eq!(
        sections.last().map(String::as_str),
        Some("flux"),
        "{sections:?}"
    );
}
