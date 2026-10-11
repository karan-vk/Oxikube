//! The Config kinds in the window: reachable from the sidebar, opened with their columns, the
//! panel above the Overview, quota bars, links, and the hidden `caBundle`.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_app::ColumnId;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::{Timeline, discovery_sets, fixtures as fx};
use serde_json::json;

use super::super::ConfigPanel;
use super::{reference, served, window};
use crate::detail::DetailView;
use crate::kinds::KindSpecs;
use crate::table::tests::fixture::cluster;

/// Every kind of the story, its fixture and the columns its table opens with.
fn matrix() -> Vec<(oxikube_domain::Resource, &'static [&'static str])> {
    vec![
        (fx::resource_quota(), &["name", "namespace", "age"]),
        (fx::limit_range(), &["name", "namespace", "age"]),
        (
            fx::hpa(),
            &[
                "name",
                "namespace",
                "reference",
                "targets",
                "min-pods",
                "max-pods",
                "replicas",
                "status",
                "age",
            ],
        ),
        (
            fx::vpa(),
            &[
                "name",
                "namespace",
                "reference",
                "mode",
                "conditions",
                "age",
            ],
        ),
        (
            fx::pdb(),
            &[
                "name",
                "namespace",
                "min-available",
                "max-unavailable",
                "allowed-disruptions",
                "current-healthy",
                "desired-healthy",
                "age",
            ],
        ),
        (
            fx::priority_class(),
            &["name", "value", "global-default", "age"],
        ),
        (fx::runtime_class(), &["name", "handler", "age"]),
        (fx::lease(), &["name", "namespace", "holder", "age"]),
        (fx::mutating_webhook(), &["name", "webhooks", "age"]),
        (fx::validating_webhook(), &["name", "webhooks", "age"]),
        (
            fx::admission_policy(),
            &["name", "validations", "param-kind", "age"],
        ),
        (
            fx::admission_policy_binding(),
            &["name", "policy", "actions", "age"],
        ),
    ]
}

#[gpui::test]
fn every_kind_opens_a_table_with_its_columns(cx: &mut TestAppContext) {
    let objects: Vec<_> = matrix().into_iter().map(|(res, _)| res).collect();
    let mut d = window(cx, objects);
    for (res, expected) in matrix() {
        let kind = served(&res.kind.kind);
        let table = d.f.open(kind);
        let columns = d.f.vcx.update(|_, cx| {
            table
                .read(cx)
                .read_rows(cx, |rows| rows.layout().visible_ids())
        });
        let expected: Vec<ColumnId> = expected.iter().map(|id| ColumnId::new(*id)).collect();
        assert_eq!(columns, expected, "{}", res.kind.kind);
        assert_eq!(
            d.f.names(&table),
            [res.meta.name.to_string()],
            "{} lists its object",
            res.kind.kind
        );
    }
}

#[gpui::test]
fn every_kind_draws_its_panel_above_the_overview(cx: &mut TestAppContext) {
    let objects: Vec<_> = matrix().into_iter().map(|(res, _)| res).collect();
    let mut d = window(cx, objects);
    for (res, _) in matrix() {
        let view = d.open(&reference(&res));
        assert!(
            d.read(&view, |v| v.kind_panel().is_some()),
            "{} has a panel",
            res.kind.kind
        );
        assert!(d.shown("detail-kind-panel"), "{}", res.kind.kind);
        assert!(d.shown("config-panel"), "{}", res.kind.kind);
        assert!(
            d.shown("detail-overview"),
            "{} keeps the generic rows",
            res.kind.kind
        );
        let panel = d.read(&view, |v| v.kind_panel().cloned()).unwrap();
        let model = d.f.vcx.update(|_, cx| {
            panel
                .downcast::<ConfigPanel>()
                .expect("a ConfigPanel")
                .update(cx, |panel, cx| panel.model(cx))
        });
        assert!(
            model.is_some_and(|m| !m.is_empty()),
            "{} has something to show",
            res.kind.kind
        );
    }
}

#[gpui::test]
fn quota_bars_render_in_proportion(cx: &mut TestAppContext) {
    let quota = fx::resource_quota();
    let mut d = window(cx, [quota.clone()]);
    d.open(&reference(&quota));
    // Resources first (limits.memory 2Gi of 8Gi, requests.cpu, gpu), then the counts
    // (count/deployments.apps, pods), so `pods` is bar 4 and `limits.memory` bar 0.
    assert!(d.shown("config-bar-0"));
    assert!(d.shown("config-bar-4"));
    let width = |d: &mut crate::detail::tests::fixture::Detail, selector: &'static str| {
        d.f.vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is on screen"))
            .size
            .width
    };
    let (memory_fill, memory_bar) = (
        width(&mut d, "config-bar-fill-0"),
        width(&mut d, "config-bar-track-0"),
    );
    let (pods_fill, pods_bar) = (
        width(&mut d, "config-bar-fill-4"),
        width(&mut d, "config-bar-track-4"),
    );
    let ratio = |fill: gpui::Pixels, track: gpui::Pixels| f32::from(fill) / f32::from(track);
    assert!(
        (ratio(memory_fill, memory_bar) - 0.25).abs() < 0.02,
        "2Gi of 8Gi"
    );
    assert!(
        (ratio(pods_fill, pods_bar) - 0.90).abs() < 0.02,
        "9 of 10 pods"
    );
}

#[gpui::test]
fn a_webhook_detail_never_shows_the_ca_bundle(cx: &mut TestAppContext) {
    let webhook = fx::mutating_webhook();
    let blob = webhook
        .get_str("/webhooks/0/clientConfig/caBundle")
        .expect("a caBundle")
        .to_owned();
    let mut d = window(cx, [webhook.clone()]);
    let view = d.open(&reference(&webhook));
    assert!(d.shown("config-panel"));
    let panel = d.read(&view, |v| v.kind_panel().cloned()).unwrap();
    let text = d.f.vcx.update(|_, cx| {
        let model = panel
            .downcast::<ConfigPanel>()
            .unwrap()
            .update(cx, |panel, cx| panel.model(cx))
            .unwrap();
        model.all_text()
    });
    assert!(text.contains("CA bundle Present"), "{text}");
    assert!(text.contains("CA bundle Absent"), "{text}");
    assert!(!text.contains(&blob));
}

#[gpui::test]
fn the_scale_target_link_opens_the_workload(cx: &mut TestAppContext) {
    let hpa = fx::hpa();
    let deployment = oxikube_testkit::deployment()
        .name("web")
        .namespace("demo")
        .build();
    let mut d = window(cx, [hpa.clone(), deployment]);
    let view = d.open(&reference(&hpa));
    assert!(d.shown("config-link-0"));
    d.f.dispatcher.clear();
    d.click("config-link-0");
    let target = ResourceRef::namespaced(
        cluster(),
        Gvk::new("apps", "v1", "Deployment"),
        "demo",
        "web",
    );
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: target.clone()
        }],
        "the link is a command, in the autoscaler's cluster and namespace"
    );
    let shown = d.drawer_view().expect("the drawer shows a detail");
    assert_ne!(shown, view);
    assert_eq!(d.read(&shown, |v: &DetailView| v.target().clone()), target);
}

#[gpui::test]
fn a_binding_links_its_cluster_scoped_policy(cx: &mut TestAppContext) {
    let binding = fx::admission_policy_binding();
    let policy = fx::admission_policy();
    let mut d = window(cx, [binding.clone(), policy]);
    d.open(&reference(&binding));
    d.f.dispatcher.clear();
    d.click("config-link-0");
    assert_eq!(
        d.f.dispatcher.sent(),
        [Command::ResourceOpen {
            target: ResourceRef::cluster_scoped(
                cluster(),
                Gvk::new(
                    "admissionregistration.k8s.io",
                    "v1",
                    "ValidatingAdmissionPolicy"
                ),
                "replica-limit",
            )
        }]
    );
}

#[gpui::test]
fn the_panel_follows_the_object_and_is_rebuilt_per_version(cx: &mut TestAppContext) {
    let pdb = fx::pdb();
    let mut json = pdb.to_value();
    json["metadata"]["resourceVersion"] = json!("3200");
    json["status"]["disruptionsAllowed"] = json!(0);
    let updated = oxikube_domain::Resource::from_json(json).unwrap();
    let mut d = window(cx, [pdb.clone()]);
    // The feed: the budget, then a new version a second later (the controller reports zero
    // allowed disruptions).
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![
            pdb.clone(),
        ])])])
        .ok_at(
            Duration::from_secs(1),
            DeltaBatch::from_deltas(vec![Delta::Applied(updated)]),
        )
        .keep_open(),
    );
    let view = d.open(&reference(&pdb));
    let panel = d
        .read(&view, |v| v.kind_panel().cloned())
        .unwrap()
        .downcast::<ConfigPanel>()
        .unwrap();
    let model = |d: &mut crate::detail::tests::fixture::Detail| {
        d.f.vcx
            .update(|_, cx| panel.update(cx, |p, cx| p.model(cx)))
            .unwrap()
    };
    let first = model(&mut d);
    let again = model(&mut d);
    assert!(
        std::sync::Arc::ptr_eq(&first, &again),
        "the same version is not rebuilt"
    );
    assert!(first.all_text().contains("Allowed disruptions 1"));
    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(1));
    d.settle();
    let after = model(&mut d);
    assert!(
        !std::sync::Arc::ptr_eq(&first, &after),
        "a new version rebuilds"
    );
    assert!(after.all_text().contains("Allowed disruptions 0"));
}

#[gpui::test]
fn the_vpa_entry_appears_only_where_its_crd_is_served(cx: &mut TestAppContext) {
    use crate::kinds::tests::row_ids;
    let with = |cx: &mut TestAppContext, kinds| {
        let mut f = crate::table::tests::fixture::Fixture::new(cx);
        f.vcx.update(|_, cx| crate::kinds::init(cx));
        let ports = f.ports();
        ports
            .access
            .set_rules(oxikube_domain::access::AccessRules::all_access());
        ports.discovery.set_kinds(kinds);
        futures::executor::block_on(f.sessions.connect(&cluster())).expect("connect");
        f.settle();
        row_ids(&mut f)
    };
    let stock = with(cx, discovery_sets::core());
    assert!(
        !stock.contains(&"config/verticalpodautoscalers".to_owned()),
        "{stock:?}"
    );
    let vpa = with(cx, discovery_sets::core_with_vpa());
    assert!(
        vpa.contains(&"config/verticalpodautoscalers".to_owned()),
        "{vpa:?}"
    );
    // The whole section, in the order the plan lists it.
    let config: Vec<&str> = vpa
        .iter()
        .filter_map(|id| id.strip_prefix("config/"))
        .collect();
    assert_eq!(
        config,
        [
            "configmaps",
            "secrets",
            "resourcequotas",
            "limitranges",
            "horizontalpodautoscalers",
            "verticalpodautoscalers",
            "poddisruptionbudgets",
            "priorityclasses",
            "runtimeclasses",
            "leases",
            "mutatingwebhookconfigurations",
            "validatingwebhookconfigurations",
            "validatingadmissionpolicies",
            "validatingadmissionpolicybindings"
        ]
    );
}

#[gpui::test]
fn every_config_kind_is_registered_with_its_panel(cx: &mut TestAppContext) {
    let mut d = window(cx, []);
    d.f.vcx.update(|_, cx| {
        for (res, _) in matrix() {
            assert!(
                !KindSpecs::detail_for(cx, &res.kind).is_generic(),
                "{} has a panel",
                res.kind.kind
            );
            let meta = KindSpecs::kind_for(cx, &res.kind);
            assert_eq!(meta.gvk.kind, res.kind.kind);
            assert!(
                meta.sidebar.is_some(),
                "{} is in the sidebar",
                res.kind.kind
            );
        }
    });
}
