//! The CRD kind (E12-S17): its detail's definition panel (names, versions with their printer
//! columns, conversion, conditions), built once per change of the CRD, and its way to the custom
//! resources; the panel's model on its own.

use std::time::Duration;

use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::{ConditionStatus, CrdSummary};
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::Timeline;
use serde_json::{Value, json};

use crate::crds::crd_gvk;
use crate::crds::tests::fixture::widget_crd_json;
use crate::detail::DetailView;
use crate::detail::tests::fixture::Detail;
use crate::kinds::KindSpecs;
use crate::kinds::crd::{CrdPanel, FlagTone, PanelModel};
use crate::table::tests::fixture::cluster;

fn crd_ref() -> ResourceRef {
    ResourceRef::cluster_scoped(cluster(), crd_gvk(), "widgets.example.com")
}

/// The Widget CRD as the API server reports it once established, with a deprecation warning on
/// `v1beta1` and a conversion webhook.
fn established(resource_version: &str) -> Value {
    let mut json = widget_crd_json();
    json["metadata"]["resourceVersion"] = json!(resource_version);
    json["spec"]["versions"][1]["deprecationWarning"] = json!("v1beta1 is going away");
    json["spec"]["conversion"] = json!({
        "strategy": "Webhook",
        "webhook": {"clientConfig": {"service": {"namespace": "ops", "name": "conv"}},
                    "conversionReviewVersions": ["v1"]}
    });
    json["status"] = json!({
        "conditions": [
            {"type": "NamesAccepted", "status": "True", "reason": "NoConflicts"},
            {"type": "Established", "status": "True", "reason": "InitialNamesAccepted"}
        ],
        "storedVersions": ["v1"]
    });
    json
}

fn panel_of(d: &mut Detail, view: &Entity<DetailView>) -> Entity<CrdPanel> {
    d.draw();
    let panel = d
        .read(view, |v| v.kind_panel().cloned())
        .expect("the CRD panel");
    panel.downcast::<CrdPanel>().expect("a CrdPanel")
}

fn model(d: &mut Detail, panel: &Entity<CrdPanel>) -> PanelModel {
    d.f.vcx
        .update(|_, cx| panel.read(cx).model().cloned())
        .expect("built")
}

#[gpui::test]
fn a_crds_detail_shows_its_names_versions_printer_columns_and_conditions(cx: &mut TestAppContext) {
    let crd = Resource::from_json(established("5")).unwrap();
    let mut d = Detail::new(cx, [crd]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let view = d.open(&crd_ref());

    assert!(d.shown("detail-kind-panel"));
    assert!(d.shown("crd-panel"));
    assert!(
        d.shown("detail-overview"),
        "the generic Overview stays below"
    );
    assert!(d.shown("detail-tab-schema"), "and the Schema tab");
    let panel = panel_of(&mut d, &view);
    let m = model(&mut d, &panel);
    assert_eq!(m.established, ConditionStatus::True);
    assert_eq!(m.names_accepted, ConditionStatus::True);
    assert_eq!(m.fact("Kind"), Some("Widget"));
    assert_eq!(m.fact("Plural"), Some("widgets"));
    assert_eq!(m.fact("Singular"), Some("widget"));
    assert_eq!(m.fact("List kind"), Some("WidgetList"));
    assert_eq!(m.fact("Short names"), Some("wd, wdg"));
    assert_eq!(m.fact("Scope"), Some("Namespaced"));
    assert_eq!(m.fact("Conversion"), Some("Webhook (ops/conv)"));
    let names: Vec<&str> = m.versions.iter().map(|v| v.name.as_ref()).collect();
    assert_eq!(names, ["v1alpha1", "v1beta1", "v1"]);
    assert_eq!(m.versions[0].flags, [("not served", FlagTone::Warn)]);
    assert_eq!(m.versions[1].flags, [("deprecated", FlagTone::Warn)]);
    assert_eq!(
        m.versions[1].warning.as_deref(),
        Some("v1beta1 is going away")
    );
    assert_eq!(m.versions[2].flags, [("storage", FlagTone::Ok)]);
    // v1's printer columns, `Owner` (priority 1) marked wide.
    let columns: Vec<(&str, bool)> = m.versions[2]
        .columns
        .iter()
        .map(|c| (c.name.as_ref(), c.wide))
        .collect();
    assert_eq!(
        columns,
        [("Size", false), ("Replicas", false), ("Owner", true)]
    );
    assert_eq!(
        m.versions[2].columns[2].text.as_ref(),
        "Owner  string  .spec.owner  (wide, priority 1)"
    );
    assert!(d.shown("crd-panel-version-v1"));
    assert!(d.shown("crd-panel-condition-Established"));
    assert!(d.shown("crd-panel-fact-Conversion"));
}

#[gpui::test]
fn the_panel_is_built_once_per_change_of_the_crd(cx: &mut TestAppContext) {
    let first = Resource::from_json(established("5")).unwrap();
    let mut changed = established("6");
    changed["status"]["conditions"][1]["status"] = json!("False");
    let changed = Resource::from_json(changed).unwrap();
    let mut d = Detail::new(cx, [first.clone()]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![first])])])
            .ok_at(
                Duration::from_secs(5),
                DeltaBatch::from_deltas(vec![Delta::Applied(changed)]),
            )
            .keep_open(),
    );
    let view = d.open(&crd_ref());
    let panel = panel_of(&mut d, &view);
    for _ in 0..5 {
        d.draw();
    }
    assert_eq!(d.f.vcx.update(|_, cx| panel.read(cx).builds()), 1);

    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(5));
    d.settle();
    assert_eq!(d.f.vcx.update(|_, cx| panel.read(cx).builds()), 2);
    assert_eq!(model(&mut d, &panel).established, ConditionStatus::False);
}

#[gpui::test]
fn the_panels_button_opens_the_custom_resources(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [Resource::from_json(established("5")).unwrap()]);
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let view = d.open(&crd_ref());
    panel_of(&mut d, &view);
    d.f.dispatcher.clear();
    d.click("crd-panel-open");
    assert!(
        d.f.dispatcher.sent().contains(&Command::CrdOpenResources {
            cluster: cluster(),
            name: "widgets.example.com".into(),
        }),
        "{:?}",
        d.f.dispatcher.sent()
    );
}

#[gpui::test]
fn the_crd_kind_is_registered_with_its_panel_and_no_sidebar_slot(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, []);
    let (panel, sidebar, offers) = d.f.vcx.update(|_, cx| {
        crate::kinds::init(cx);
        let meta = KindSpecs::kind_for(cx, &crd_gvk());
        (
            !KindSpecs::detail_for(cx, &crd_gvk()).is_generic(),
            meta.sidebar,
            meta.offers(oxikube_domain::command::CommandId::CRD_OPEN_RESOURCES),
        )
    });
    assert!(panel);
    assert!(sidebar.is_none(), "listed by Custom Resources' Definitions");
    assert!(offers);
}

#[test]
fn the_model_leaves_out_empty_facts_and_reads_a_crd_that_serves_nothing() {
    let mut json = widget_crd_json();
    json["spec"]["names"] = json!({"plural": "widgets", "kind": "Widget"});
    for version in json["spec"]["versions"].as_array_mut().unwrap() {
        version["served"] = json!(false);
    }
    let crd = CrdSummary::from_resource(&Resource::from_json(json).unwrap()).unwrap();
    let m = PanelModel::new(&crd);
    assert_eq!(m.fact("Singular"), None);
    assert_eq!(m.fact("Short names"), None);
    assert_eq!(m.fact("Conversion"), Some("None"));
    assert!(!m.can_open, "nothing served: no table to open");
    assert_eq!(m.established, ConditionStatus::Unknown);
    assert!(
        m.versions
            .iter()
            .all(|v| v.flags.contains(&("not served", FlagTone::Warn)))
    );
    assert!(m.versions[0].columns.is_empty());
}
