//! The detail's tabs: each shows its fixture, values user / all, masking and the explicit reveal.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{AppContext as _, TestAppContext};
use oxikube_domain::command::Command;
use oxikube_domain::helm::{ChartRef, ReleaseStatus, Revision, Values};
use serde_json::json;

use super::{Data, Harness, at, full, object, release, release_ref};
use crate::releases::{Load, ObjectState, ValuesMode};

fn web() -> Data {
    let mut data = Data {
        releases: vec![release("apps", "web", ReleaseStatus::Deployed, 3)],
        ..Data::default()
    };
    data.user_values.insert(
        "web".into(),
        Values::from_json(json!({"replicaCount": 3, "auth": {"adminPassword": "hunter2"}})),
    );
    data.all_values.insert(
        "web".into(),
        Values::from_json(json!({
            "replicaCount": 3,
            "image": {"tag": "1.25"},
            "auth": {"adminPassword": "hunter2", "username": "admin"},
        })),
    );
    data.manifest.insert(
        "web".into(),
        "---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: creds\ndata:\n  k: c2VjcmV0\n"
            .into(),
    );
    data.notes.insert(
        "web".into(),
        Some("1. Visit https://example.com/web\n2. Or http://localhost:8080/ locally.".into()),
    );
    data.history.insert(
        "web".into(),
        (1..=3)
            .rev()
            .map(|n| Revision {
                revision: n,
                status: if n == 3 {
                    ReleaseStatus::Deployed
                } else {
                    ReleaseStatus::Superseded
                },
                chart: ChartRef::new("web", format!("1.{n}.0")),
                app_version: Some(format!("2.{n}")),
                updated: at(1_700_000_000 + i64::from(n)),
                description: if n == 1 {
                    "Install complete"
                } else {
                    "Upgrade complete"
                }
                .into(),
            })
            .collect(),
    );
    data.objects = vec![
        object("Deployment", Some("apps"), "web", ObjectState::Healthy),
        object("Pod", Some("apps"), "web-1", ObjectState::Unhealthy),
        object("Service", Some("apps"), "web", ObjectState::Present),
        object("ConfigMap", Some("apps"), "gone", ObjectState::Missing),
        object("ClusterRole", None, "web", ObjectState::Present),
        object("Widget", Some("apps"), "w", ObjectState::NotServed),
    ];
    data
}

fn open(cx: &mut TestAppContext) -> Harness {
    let mut h = Harness::open(cx, web(), full());
    h.select(0);
    h
}

fn code_text(
    h: &mut Harness,
    pick: impl FnOnce(&crate::releases::ReleaseDetail) -> gpui::Entity<oxikube_ui::code_view::CodeView>,
) -> String {
    let code = h.detail(pick);
    h.vcx.read(|cx| {
        code.read(cx)
            .text()
            .map(|t| t.to_string())
            .unwrap_or_default()
    })
}

#[gpui::test]
fn resources_show_health_flag_missing_and_open_the_object_on_click(cx: &mut TestAppContext) {
    let mut h = open(cx);
    assert!(h.is_laid_out("helm-release-resources"));
    h.detail(|d| {
        assert_eq!(d.resources.load, Load::Ready);
        let states: Vec<_> = d.resources.objects.iter().map(|o| o.state).collect();
        assert_eq!(
            states,
            [
                ObjectState::Healthy,
                ObjectState::Unhealthy,
                ObjectState::Present,
                ObjectState::Missing,
                ObjectState::Present,
                ObjectState::NotServed
            ]
        );
    });
    assert!(h.is_laid_out("helm-release-object:Deployment/web"));
    assert!(
        h.is_laid_out("helm-release-object:ConfigMap/gone"),
        "missing is flagged, never hidden"
    );
    h.click("helm-release-object:Pod/web-1");
    let sent = h.bus.sent();
    assert!(
        matches!(
            sent.as_slice(),
            [Command::ResourceOpen { target }] if &*target.name == "web-1" && target.gvk.kind.as_ref() == "Pod"
        ),
        "{sent:?}"
    );
    // A missing object has nothing to open.
    h.click("helm-release-object:ConfigMap/gone");
    assert_eq!(h.bus.sent().len(), 1);
}

#[gpui::test]
fn resources_follow_the_cluster_while_the_tab_shows(cx: &mut TestAppContext) {
    let mut h = open(cx);
    let state = |h: &mut Harness| h.detail(|d| d.resources.objects[3].state);
    assert_eq!(state(&mut h), ObjectState::Missing);
    // The object appears in the cluster; the tab notices within its tick.
    h.backend.live_objects.lock()[3].state = ObjectState::Healthy;
    h.vcx
        .executor()
        .advance_clock(crate::releases::OBJECTS_TICK + Duration::from_millis(10));
    h.settle();
    assert_eq!(state(&mut h), ObjectState::Healthy);
}

#[gpui::test]
fn leaving_the_resources_tab_stops_its_tick(cx: &mut TestAppContext) {
    let mut h = open(cx);
    assert!(h.detail(|d| d.resources.tick.is_some()));
    h.click("helm-release-tab:notes");
    assert!(h.detail(|d| d.resources.tick.is_none()));
    h.click("helm-release-tab:resources");
    assert!(
        h.detail(|d| d.resources.tick.is_some()),
        "back on the tab, it follows again"
    );
}

#[gpui::test]
fn values_show_the_users_values_then_all_of_them(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.click("helm-release-tab:values");
    assert!(h.is_laid_out("helm-release-values"));
    let text = code_text(&mut h, |d| d.values.code.clone());
    assert!(text.contains("replicaCount: 3"), "{text}");
    assert!(!text.contains("image:"), "user values only: {text}");
    h.detail(|d| assert_eq!(d.mode, ValuesMode::User));
    assert!(h.calls().contains(&"values:web:3:false".to_owned()));

    h.click("helm-release-mode:All");
    h.detail(|d| assert_eq!(d.mode, ValuesMode::All));
    let text = code_text(&mut h, |d| d.values.code.clone());
    assert!(
        text.contains("image:") && text.contains("username: admin"),
        "{text}"
    );
    assert!(h.calls().contains(&"values:web:3:true".to_owned()));

    h.click("helm-release-mode:User");
    let text = code_text(&mut h, |d| d.values.code.clone());
    assert!(!text.contains("image:"), "back to the user's: {text}");
}

#[gpui::test]
fn an_empty_values_tree_says_the_chart_defaults_apply(cx: &mut TestAppContext) {
    let mut data = web();
    data.user_values.insert("web".into(), Values::default());
    let mut h = Harness::open(cx, data, full());
    h.select(0);
    h.click("helm-release-tab:values");
    assert!(h.is_laid_out("helm-release-values-empty"));
}

#[gpui::test]
fn credential_looking_values_are_masked_until_the_reveal_command_ran(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.click("helm-release-tab:values");
    let masked = code_text(&mut h, |d| d.values.code.clone());
    assert!(!masked.contains("hunter2"), "{masked}");
    assert!(masked.contains("********"));
    h.detail(|d| {
        assert_eq!(
            (d.values_facts.sensitive, d.values_facts.revealed),
            (1, false)
        );
    });
    assert!(h.is_laid_out("helm-release-masking"));

    // The button sends the command; it does not reveal anything by itself.
    h.click("helm-release-reveal");
    let sent = h.bus.sent();
    assert!(
        matches!(
            sent.as_slice(),
            [Command::HelmRevealValues { namespace, name, reveal: true, .. }] if namespace == "apps" && name == "web"
        ),
        "{sent:?}"
    );
    let still = code_text(&mut h, |d| d.values.code.clone());
    assert!(
        !still.contains("hunter2"),
        "no reveal before the command ran"
    );

    // The command's handler (the app service) grants it; the tab reads again and shows it.
    h.backend.grant(&release_ref("web"), true);
    h.settle();
    let clear = code_text(&mut h, |d| d.values.code.clone());
    assert!(clear.contains("hunter2"), "{clear}");
    assert!(h.detail(|d| d.values_facts.revealed));
    h.click("helm-release-reveal");
    assert!(matches!(
        h.bus.sent().last(),
        Some(Command::HelmRevealValues { reveal: false, .. })
    ));
}

#[gpui::test]
fn closing_the_detail_hides_what_was_revealed(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.click("helm-release-tab:values");
    h.backend.grant(&release_ref("web"), true);
    h.settle();
    assert!(h.backend.data.lock().revealed.contains(&release_ref("web")));
    let view = h.view.clone();
    h.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.close_detail(cx)));
    h.settle();
    assert!(h.backend.data.lock().revealed.is_empty());
}

#[gpui::test]
fn the_manifest_masks_secret_data_until_revealed(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.click("helm-release-tab:manifest");
    assert!(h.is_laid_out("helm-release-manifest"));
    let text = code_text(&mut h, |d| d.manifest.code.clone());
    assert!(
        text.contains("kind: Secret") && !text.contains("c2VjcmV0"),
        "{text}"
    );
    assert!(h.detail(|d| d.manifest_masked));
    h.backend.grant(&release_ref("web"), true);
    h.settle();
    let text = code_text(&mut h, |d| d.manifest.code.clone());
    assert!(text.contains("c2VjcmV0"));
    assert!(!h.detail(|d| d.manifest_masked));
}

#[gpui::test]
fn notes_show_plain_text_and_their_links(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.click("helm-release-tab:notes");
    assert!(h.is_laid_out("helm-release-notes"));
    let text = code_text(&mut h, |d| d.notes.code.clone());
    assert!(text.contains("1. Visit https://example.com/web"));
    assert!(h.is_laid_out("helm-release-links"));
    h.detail(|d| {
        assert_eq!(
            d.links,
            ["https://example.com/web", "http://localhost:8080/"]
        );
    });
}

#[gpui::test]
fn a_chart_without_notes_says_so(cx: &mut TestAppContext) {
    let mut data = web();
    data.notes = HashMap::from([("web".to_owned(), None)]);
    let mut h = Harness::open(cx, data, full());
    h.select(0);
    h.click("helm-release-tab:notes");
    assert!(h.is_laid_out("helm-release-notes-empty"));
}

#[gpui::test]
fn history_lists_every_revision_newest_first(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.click("helm-release-tab:history");
    assert!(h.is_laid_out("helm-release-history"));
    for n in 1..=3 {
        assert!(
            h.is_laid_out(&format!("helm-release-revision:{n}")),
            "revision {n}"
        );
    }
    h.detail(|d| {
        let revisions: Vec<_> = d.history.revisions.iter().map(|r| r.revision).collect();
        assert_eq!(revisions, [3, 2, 1]);
        assert_eq!(d.history.revisions[0].status, ReleaseStatus::Deployed);
        assert_eq!(d.history.revisions[2].description, "Install complete");
    });
}

#[gpui::test]
fn tabs_read_only_when_first_opened_and_keep_what_they_read(cx: &mut TestAppContext) {
    let mut h = open(cx);
    for key in [
        "values",
        "manifest",
        "notes",
        "history",
        "resources",
        "values",
    ] {
        h.click(&format!("helm-release-tab:{key}"));
    }
    let calls = h.calls();
    let count = |prefix: &str| calls.iter().filter(|c| c.starts_with(prefix)).count();
    assert_eq!(count("values:"), 1);
    assert_eq!(count("manifest:"), 1);
    assert_eq!(count("notes:"), 1);
    assert_eq!(count("history:"), 1);
    assert_eq!(count("objects:"), 1);
}

#[gpui::test]
fn a_failing_tab_says_why_and_the_others_still_work(cx: &mut TestAppContext) {
    let mut data = web();
    data.user_values.clear();
    let mut h = Harness::open(cx, data, full());
    h.select(0);
    h.click("helm-release-tab:values");
    assert!(h.is_laid_out("helm-release-values-failed"));
    h.click("helm-release-tab:history");
    assert!(h.is_laid_out("helm-release-history"));
}

#[gpui::test]
fn a_read_in_flight_is_not_left_waiting_when_the_values_mode_changes(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, web(), full());
    let view = h.view.clone();
    // Resources has been asked for and has not landed when the user flips the mode.
    h.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            let release = v.table().read(cx, |d| d.release(0).cloned()).expect("row");
            v.open_detail(release, cx);
            v.set_tab(crate::releases::Tab::Values, cx);
            v.toggle_values_mode(cx);
            let detail = v.detail.as_ref().expect("detail");
            assert!(
                !matches!(detail.load(crate::releases::Tab::Resources), Load::Loading),
                "a read the generation would discard must not stay Loading"
            );
        });
    });
    h.settle();
    h.click("helm-release-tab:resources");
    h.settle();
    assert!(matches!(
        h.detail(|d| d.load(crate::releases::Tab::Resources).clone()),
        Load::Ready
    ));
}

#[gpui::test]
fn closing_the_page_hides_what_was_revealed(cx: &mut TestAppContext) {
    let mut h = open(cx);
    h.backend.grant(&release_ref("web"), true);
    h.settle();
    assert!(h.backend.data.lock().revealed.contains(&release_ref("web")));
    // The page entity goes away with its tab; the reveal must go with it.
    let (backend, gate, bus) = (h.backend.clone(), h.gate.clone(), h.bus.clone());
    let view = h.vcx.update(|window, cx| {
        let deps = crate::releases::ReleasesDeps {
            backend,
            dispatcher: std::rc::Rc::new(bus),
            gate,
        };
        cx.new(|cx| crate::releases::HelmReleasesView::new(deps, window, cx))
    });
    h.settle();
    h.vcx.update(|_, cx| {
        view.update(cx, |v, cx| {
            let release = v.table().read(cx, |d| d.release(0).cloned()).expect("row");
            v.open_detail(release, cx);
        });
    });
    h.backend.grant(&release_ref("web"), true);
    drop(view);
    h.settle();
    assert!(!h.backend.data.lock().revealed.contains(&release_ref("web")));
}
