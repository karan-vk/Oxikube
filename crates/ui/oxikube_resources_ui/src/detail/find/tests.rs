//! Find in the YAML and Describe text: `/`, typing, `n` / `N` with wrap, the keys being text
//! while the field has the focus, errors, and the commands the keys send.

use std::time::Duration;

use gpui::{Entity, TestAppContext};
use oxikube_domain::command::Command;
use oxikube_ports::{Delta, DeltaBatch, DescribeOutput, DescribeSource};
use oxikube_testkit::Timeline;

use crate::detail::tests::fixture::{Detail, edited, pod_ref, web_pod};
use crate::detail::{DetailTab, DetailView};
use crate::table::tests::p;

/// A window with the pods `a`, `b`, `c` listed and the detail of `b` open on its YAML tab, the
/// focus in the drawer (Enter on the row).
fn drawer_on_yaml(cx: &mut TestAppContext) -> (Detail, Entity<DetailView>) {
    let mut d = Detail::new(cx, ["a", "b", "c"].map(|name| p("shop", name, "1")));
    let table = d.f.open_pods();
    d.f.keys(&table, "j j enter");
    d.settle();
    let view = d.drawer_view().expect("the drawer shows a detail");
    press(&mut d, "y");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml);
    (d, view)
}

fn press(d: &mut Detail, keys: &str) {
    d.f.vcx.simulate_keystrokes(keys);
    d.settle();
}

fn yaml(d: &mut Detail, view: &Entity<DetailView>) -> String {
    d.read(view, |v| v.yaml().map(str::to_owned)).expect("YAML")
}

/// How many times `needle` appears in `text`, ignoring case.
fn count(text: &str, needle: &str) -> usize {
    text.to_lowercase().matches(&needle.to_lowercase()).count()
}

fn focused(d: &mut Detail, view: &Entity<DetailView>) -> bool {
    d.f.vcx.update(|window, cx| {
        gpui::Focusable::focus_handle(view.read(cx), cx).contains_focused(window, cx)
    })
}

#[gpui::test]
fn slash_opens_the_field_and_typing_finds_and_colours_the_matches(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    assert!(!d.read(&view, |v| v.find_open()));
    assert!(!d.shown("detail-find"));

    press(&mut d, "/");
    assert!(d.read(&view, |v| v.find_open()));
    assert!(d.shown("detail-find"));
    assert!(d.shown("detail-find-input"));
    // Right after `/` the keys are text, not tab and row keys.
    press(&mut d, "s h o p");
    let text = yaml(&mut d, &view);
    let want = count(&text, "shop");
    assert!(want >= 1, "the fixture mentions the namespace: {text}");
    assert_eq!(d.read(&view, |v| v.find_text_typed().to_owned()), "shop");
    assert_eq!(d.read(&view, |v| v.find_match_count()), want);
    assert_eq!(
        d.read(&view, |v| v.find_position()),
        Some((1, want)),
        "the first match at or after the top is current"
    );
    assert_eq!(
        d.read(&view, |v| v.find_label()).as_deref(),
        Some(format!("1 / {want}").as_str())
    );
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml);
}

#[gpui::test]
fn each_character_typed_keeps_the_first_match_current(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    press(&mut d, "/");
    // One key at a time, with the scan settling in between: a longer pattern whose first match
    // starts where the last one's did must not step on to the next match.
    let mut seen = Vec::new();
    for key in ["n", "a", "m", "e"] {
        press(&mut d, key);
        let total = d.read(&view, |v| v.find_match_count());
        assert!(total >= 2, "{key}: {total} matches make the step visible");
        seen.push(d.read(&view, |v| v.find_position()).map(|(n, _)| n));
    }
    assert_eq!(seen, vec![Some(1); 4]);
    // A backspace widens the pattern again: still the first.
    press(&mut d, "backspace");
    assert_eq!(
        d.read(&view, |v| v.find_position()).map(|(n, _)| n),
        Some(1)
    );
}

#[gpui::test]
fn n_and_shift_n_step_through_the_matches_and_wrap_around(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    press(&mut d, "/ e");
    let total = d.read(&view, |v| v.find_match_count());
    assert!(total >= 3, "{total} matches");
    // Enter keeps the matches and returns the keys to the detail.
    press(&mut d, "enter");
    assert!(focused(&mut d, &view));
    assert_eq!(d.read(&view, |v| v.find_position()), Some((1, total)));

    press(&mut d, "n");
    assert_eq!(d.read(&view, |v| v.find_position()), Some((2, total)));
    for _ in 2..total {
        press(&mut d, "n");
    }
    assert_eq!(d.read(&view, |v| v.find_position()), Some((total, total)));
    press(&mut d, "n");
    assert_eq!(
        d.read(&view, |v| v.find_position()),
        Some((1, total)),
        "n at the last match wraps to the first"
    );
    press(&mut d, "shift-n");
    assert_eq!(
        d.read(&view, |v| v.find_position()),
        Some((total, total)),
        "N at the first match wraps to the last"
    );
    press(&mut d, "shift-n");
    assert_eq!(
        d.read(&view, |v| v.find_position()),
        Some((total - 1, total))
    );
}

#[gpui::test]
fn the_keys_send_the_resource_commands_for_the_object_shown(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    let target = d.read(&view, |v| v.target().clone());
    d.f.dispatcher.clear();
    press(&mut d, "/");
    press(&mut d, "s h o p enter");
    d.f.dispatcher.clear();
    press(&mut d, "n");
    press(&mut d, "shift-n");
    assert_eq!(
        d.f.dispatcher.sent(),
        [
            Command::ResourceNextMatch {
                target: target.clone()
            },
            Command::ResourcePreviousMatch { target },
        ]
    );
}

#[gpui::test]
fn slash_sends_resource_find_once_and_the_echo_does_not_refocus(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    let target = d.read(&view, |v| v.target().clone());
    d.f.dispatcher.clear();
    press(&mut d, "/ s h o p enter");
    assert_eq!(
        d.f.dispatcher
            .sent()
            .iter()
            .filter(|c| matches!(c, Command::ResourceFind { .. }))
            .collect::<Vec<_>>(),
        [&Command::ResourceFind {
            target,
            pattern: None
        }]
    );
    assert!(
        focused(&mut d, &view),
        "enter returned to the detail and the command's echo did not take the focus back"
    );
    assert_eq!(d.read(&view, |v| v.find_text_typed().to_owned()), "shop");
}

#[gpui::test]
fn bare_keys_are_text_while_the_field_has_the_focus(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    let before = d.read(&view, |v| v.target().name.to_string());
    press(&mut d, "/ j k 1 2 y");
    assert_eq!(d.read(&view, |v| v.find_text_typed().to_owned()), "jk12y");
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml, "no tab switch");
    assert_eq!(
        d.read(&view, |v| v.target().name.to_string()),
        before,
        "j / k did not step the table"
    );
}

#[gpui::test]
fn escape_closes_the_field_and_a_second_escape_closes_the_drawer(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    press(&mut d, "/ s h o p");
    assert!(d.read(&view, |v| v.find_match_count()) > 0);
    press(&mut d, "escape");
    assert!(!d.read(&view, |v| v.find_open()), "the field is closed");
    assert_eq!(d.read(&view, |v| v.find_match_count()), 0, "and forgotten");
    assert!(d.drawer_view().is_some(), "the drawer is still open");
    assert!(focused(&mut d, &view), "the keys are the detail's again");
    press(&mut d, "escape");
    assert!(d.drawer_view().is_none(), "escape closes the drawer");
}

#[gpui::test]
fn an_invalid_pattern_keeps_the_last_matches_and_says_why(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    press(&mut d, "/ s h o p");
    let total = d.read(&view, |v| v.find_match_count());
    press(&mut d, "(");
    assert!(d.read(&view, |v| v.find_error().is_some()));
    assert_eq!(d.read(&view, |v| v.find_match_count()), total);
    assert!(d.shown("detail-find-error"));
    press(&mut d, "backspace");
    assert!(d.read(&view, |v| v.find_error().is_none()));
    assert!(!d.shown("detail-find-error"));
}

#[gpui::test]
fn no_match_says_so_and_n_does_nothing(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    press(&mut d, "/ z z z q q");
    assert_eq!(d.read(&view, |v| v.find_match_count()), 0);
    assert_eq!(
        d.read(&view, |v| v.find_label()).as_deref(),
        Some("No matches")
    );
    press(&mut d, "enter");
    press(&mut d, "n");
    assert_eq!(d.read(&view, |v| v.find_position()), None);
}

#[gpui::test]
fn a_regex_pattern_matches_case_insensitively(cx: &mut TestAppContext) {
    let (mut d, view) = drawer_on_yaml(cx);
    let text = yaml(&mut d, &view);
    let want = count(&text, "kind:") + count(&text, "name:");
    d.update(&view, |v, cx| {
        v.edit_find("KIND:|name:", cx);
    });
    assert_eq!(d.read(&view, |v| v.find_match_count()), want);
}

#[gpui::test]
fn find_on_the_overview_shows_the_yaml_tab(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    let view = d.open(&pod_ref("web-0"));
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Overview);
    d.f.vcx.update(|window, cx| {
        view.update(cx, |v, cx| v.find(Some("app"), window, cx));
    });
    d.settle();
    assert_eq!(d.read(&view, |v| v.tab()), DetailTab::Yaml);
    assert!(d.read(&view, |v| v.find_match_count()) > 0);
}

#[gpui::test]
fn a_new_version_of_the_object_is_searched_again(cx: &mut TestAppContext) {
    let pod = web_pod();
    let updated = edited(pod.clone(), |json| {
        json["metadata"]["resourceVersion"] = "999".into();
        json["metadata"]["labels"]["extra"] = "web-extra".into();
    });
    let mut d = Detail::new(cx, []);
    d.f.ports().resources.script().watch.push_ok(
        Timeline::immediate([DeltaBatch::from_deltas(vec![Delta::Restarted(vec![pod])])])
            .ok_at(
                Duration::from_secs(1),
                DeltaBatch::from_deltas(vec![Delta::Applied(updated)]),
            )
            .keep_open(),
    );
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-yaml");
    d.f.vcx.update(|window, cx| {
        view.update(cx, |v, cx| v.find(Some("web"), window, cx));
    });
    d.settle();
    let before = d.read(&view, |v| v.find_match_count());
    d.f.ports()
        .resources
        .clock()
        .advance(Duration::from_secs(1));
    d.settle();
    d.draw();
    let text = yaml(&mut d, &view);
    assert!(text.contains("web-extra"), "{text}");
    let after = d.read(&view, |v| v.find_match_count());
    assert_eq!(after, count(&text, "web"));
    assert!(after > before, "{before} -> {after}");
}

#[gpui::test]
fn the_describe_tab_is_searched_too(cx: &mut TestAppContext) {
    let mut d = Detail::new(cx, [web_pod()]);
    d.f.ports()
        .describe
        .script()
        .describe
        .push_ok(DescribeOutput {
            text: "Name:  web-0\nNode:  worker-1\nLabels: app=web\nweb again\n".into(),
            source: DescribeSource::Native,
        });
    let view = d.open(&pod_ref("web-0"));
    d.click("detail-tab-describe");
    d.f.vcx.update(|window, cx| {
        view.update(cx, |v, cx| v.find(Some("web"), window, cx));
    });
    d.settle();
    assert_eq!(d.read(&view, |v| v.find_match_count()), 3);
    d.update(&view, |v, cx| v.next_match(cx));
    assert_eq!(d.read(&view, |v| v.find_position()), Some((2, 3)));
    // Switching to the YAML tab searches that text instead.
    d.click("detail-tab-yaml");
    let text = yaml(&mut d, &view);
    assert_eq!(d.read(&view, |v| v.find_match_count()), count(&text, "web"));
}
