//! The keyboard: arrows and `hjkl` over the tiles, `enter` drilling into the right table, `esc`
//! going back, a click, and the keymap as data (defaults on every platform, rebinding).

use gpui::TestAppContext;
use oxikube_assets::{KeymapPlatform, default_keymap};
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_keymap::file::parse_keymap;
use oxikube_keymap::{ActionRegistry, KeymapAction, KeymapLayer, bindings_for_action_name};
use oxikube_testkit::pod;

use super::{Fixture, cluster, no_grace};

/// The shipped keys of the `Pulses` context.
const DEFAULTS: [(&str, &str); 13] = [
    ("left", "pulses::SelectLeft"),
    ("h", "pulses::SelectLeft"),
    ("right", "pulses::SelectRight"),
    ("l", "pulses::SelectRight"),
    ("up", "pulses::SelectUp"),
    ("k", "pulses::SelectUp"),
    ("down", "pulses::SelectDown"),
    ("j", "pulses::SelectDown"),
    ("home", "pulses::SelectFirst"),
    ("end", "pulses::SelectLast"),
    ("enter", "pulses::Open"),
    ("r", "pulses::Refresh"),
    ("escape", "pulses::Close"),
];

fn open(cx: &mut TestAppContext) -> Fixture {
    let mut fx = Fixture::open_seeded(cx, no_grace(), |ports| {
        ports
            .resources
            .insert(pod().namespace("a").name("p1").running().build());
    });
    fx.tick();
    fx.draw();
    fx
}

fn open_list(gvk: Gvk) -> Command {
    Command::ResourceOpenList {
        cluster: cluster(),
        gvk,
    }
}

#[gpui::test]
fn arrow_keys_and_hjkl_move_over_the_tiles(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    assert_eq!(fx.selected(), "cluster-cpu");
    fx.keys("right");
    assert_eq!(fx.selected(), "cluster-memory");
    fx.keys("l");
    assert_eq!(fx.selected(), "nodes");
    fx.keys("down");
    assert_eq!(fx.selected(), "statefulsets", "a row is four tiles");
    fx.keys("j");
    assert_eq!(fx.selected(), "persistentvolumes");
    fx.keys("left");
    assert_eq!(fx.selected(), "cronjobs");
    fx.keys("h");
    assert_eq!(fx.selected(), "jobs");
    fx.keys("up");
    assert_eq!(fx.selected(), "pods");
    fx.keys("k");
    assert_eq!(fx.selected(), "cluster-cpu");
}

#[gpui::test]
fn home_and_end_jump_to_the_first_and_last_tile(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    fx.keys("end");
    assert_eq!(fx.selected(), "serviceaccounts");
    fx.keys("home");
    assert_eq!(fx.selected(), "cluster-cpu");
}

#[gpui::test]
fn a_move_off_the_edge_stays_put(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    fx.keys("left");
    fx.keys("up");
    assert_eq!(fx.selected(), "cluster-cpu");
    fx.keys("end");
    fx.keys("right");
    fx.keys("down");
    assert_eq!(fx.selected(), "serviceaccounts");
}

#[gpui::test]
fn enter_drills_into_the_table_of_the_selected_kind(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    fx.keys("j");
    assert_eq!(fx.selected(), "pods");
    fx.keys("enter");
    assert_eq!(fx.recorder.sent(), [open_list(Gvk::new("", "v1", "Pod"))]);
}

#[gpui::test]
fn every_tile_drills_into_its_own_table(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    let page = fx.page.clone();
    for index in 0..16 {
        fx.vcx.update(|_, cx| {
            page.update(cx, |page, _| {
                page.model.select(index);
            });
        });
        fx.keys("enter");
    }
    let sent = fx.recorder.sent();
    assert_eq!(sent.len(), 16);
    let node = open_list(Gvk::new("", "v1", "Node"));
    assert_eq!(sent[0], node, "the CPU gauge drills into Nodes");
    assert_eq!(sent[1], node, "and so does memory");
    assert_eq!(sent[2], node);
    assert_eq!(sent[3], open_list(Gvk::new("", "v1", "Namespace")));
    assert_eq!(sent[4], open_list(Gvk::new("", "v1", "Pod")));
    assert_eq!(sent[5], open_list(Gvk::new("apps", "v1", "Deployment")));
    assert_eq!(sent[8], open_list(Gvk::new("batch", "v1", "Job")));
    assert_eq!(
        sent[12],
        open_list(Gvk::new("autoscaling", "v2", "HorizontalPodAutoscaler"))
    );
    assert_eq!(sent[15], open_list(Gvk::new("", "v1", "ServiceAccount")));
}

#[gpui::test]
fn a_click_selects_the_tile_and_opens_its_table(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    let bounds = fx
        .vcx
        .debug_bounds("pulse-tile-deployments")
        .expect("drawn");
    fx.vcx
        .simulate_click(bounds.center(), gpui::Modifiers::none());
    fx.vcx.run_until_parked();
    assert_eq!(fx.selected(), "deployments");
    assert_eq!(
        fx.recorder.sent(),
        [open_list(Gvk::new("apps", "v1", "Deployment"))]
    );
}

#[gpui::test]
fn escape_closes_the_page_and_goes_back(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    assert_eq!(fx.ws.read_with(&fx.vcx, |ws, _| ws.items().count()), 1);
    fx.keys("escape");
    assert_eq!(fx.ws.read_with(&fx.vcx, |ws, _| ws.items().count()), 0);
    assert!(fx.recorder.sent().is_empty(), "closing sends nothing");
}

#[gpui::test]
fn the_selected_tile_is_marked_and_moving_keeps_every_tile_in_place(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    let before = fx.vcx.debug_bounds("pulse-tile-pods").expect("drawn");
    fx.keys("l l j");
    fx.draw();
    let after = fx.vcx.debug_bounds("pulse-tile-pods").expect("drawn");
    assert_eq!(before, after, "selecting does not move the tile");
}

fn defaults_of(platform: KeymapPlatform) -> Vec<(String, String)> {
    let parsed = parse_keymap(default_keymap(platform), KeymapLayer::Default).expect("a keymap");
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let sections: Vec<_> = parsed
        .sections
        .iter()
        .filter(|(_, section)| section.context_expr() == Some("Pulses"))
        .collect();
    assert_eq!(sections.len(), 1, "one `Pulses` section on {platform:?}");
    sections[0]
        .1
        .bindings
        .iter()
        .map(
            |(key, value)| match KeymapAction::from_json(value).expect("a binding") {
                KeymapAction::Action { name, data: None } => (key.clone(), name),
                other => panic!("{key}: {other:?}"),
            },
        )
        .collect()
}

#[test]
fn every_platform_ships_the_same_pulses_keys() {
    let expected: Vec<(String, String)> = DEFAULTS
        .iter()
        .map(|(key, name)| ((*key).to_owned(), (*name).to_owned()))
        .collect();
    for platform in [
        KeymapPlatform::MacOs,
        KeymapPlatform::Linux,
        KeymapPlatform::Windows,
    ] {
        assert_eq!(defaults_of(platform), expected, "{platform:?}");
    }
}

#[gpui::test]
fn the_default_keys_name_actions_the_page_handles(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    let focus = fx.read(|p| p.focus.clone());
    for (key, name) in DEFAULTS {
        fx.vcx.update(|window, cx| {
            assert!(
                ActionRegistry::from_app(cx).contains(name),
                "{name} is registered"
            );
            let infos = bindings_for_action_name(cx, name, None);
            let info = infos
                .iter()
                .find(|info| info.keystrokes_text() == key)
                .unwrap_or_else(|| panic!("{key} is bound to {name}: {infos:?}"));
            assert_eq!(info.context.as_deref(), Some("Pulses"));
            let action = cx.build_action(name, None).expect("the action builds");
            assert!(
                window.is_action_available_in(action.as_ref(), &focus),
                "the page handles {name}"
            );
        });
    }
}

#[gpui::test]
fn a_users_keymap_rebinds_the_pulses_keys(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    fx.vcx.update(|_, cx| {
        oxikube_keymap::reload_user_keymap(
            cx,
            r#"[{ "context": "Pulses", "bindings": { "j": null, "n": "pulses::SelectDown" } }]"#,
        );
    });
    fx.vcx.run_until_parked();
    fx.keys("j");
    assert_eq!(fx.selected(), "cluster-cpu", "j is unbound");
    fx.keys("n");
    assert_eq!(fx.selected(), "pods", "n moves down now");
}
