//! The user's `keymap.json` (E11-S08): sections layered over the defaults, `null` to unbind,
//! validation with line numbers and one summarising notification, reload from disk, conflicts and
//! the cost of applying a big file.
//!
//! Everything runs without the file watcher (GPUI's scheduler forbids its thread): the text
//! arrives through `reload_user_keymap` or `reload`, which is what the watcher's callback does.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use common::*;
use gpui::TestAppContext;
use oxikube_keymap::{
    KeybindSource, KeymapDiagnosticsEvent, KeymapLayer, KeymapOptions, KeymapPlatform,
    KeymapProblem, ParsedUserKeymap, bindings_for_action_name, conflicts, init_with_dir,
    init_with_text, reload, reload_user_keymap, subscribe_diagnostics, user_diagnostics,
    user_keymap_file,
};

fn mac() -> KeymapOptions {
    KeymapOptions {
        platform: KeymapPlatform::MacOs,
        vim: false,
    }
}

fn install(cx: &mut TestAppContext, user: &str) {
    cx.update(|cx| init_with_text(user, mac(), cx));
}

type Events = Rc<RefCell<Vec<KeymapDiagnosticsEvent>>>;

/// Records the notification events; hold the subscription for as long as they are wanted.
fn listen(cx: &mut TestAppContext) -> (Events, gpui::Subscription) {
    let events = Events::default();
    let sink = events.clone();
    let subscription = cx.update(|cx| {
        subscribe_diagnostics(cx, move |event, _| sink.borrow_mut().push(event.clone()))
    });
    (events, subscription)
}

#[gpui::test]
fn a_user_file_layers_over_the_defaults_and_null_unbinds(cx: &mut TestAppContext) {
    install(
        cx,
        r#"[{"bindings": {"cmd-q": "kmtest::Alpha", "cmd-shift-p": null, "cmd-j": "kmtest::Beta"}}]"#,
    );
    let (window, log) = probe(cx, "Pane");
    assert_eq!(
        press(cx, window, &log, "cmd-q"),
        ["Alpha"],
        "overrides the default"
    );
    assert!(
        press(cx, window, &log, "cmd-shift-p").is_empty(),
        "null unbinds the default"
    );
    assert_eq!(press(cx, window, &log, "cmd-j"), ["Beta"], "adds a binding");
    assert_eq!(cx.read(user_diagnostics), []);
}

#[gpui::test]
fn null_on_a_binding_that_does_not_exist_is_silent(cx: &mut TestAppContext) {
    install(
        cx,
        r#"[{"context": "Nowhere", "bindings": {"ctrl-alt-shift-z": null}}, {"bindings": {"f13": null}}]"#,
    );
    assert_eq!(cx.read(user_diagnostics), []);
}

#[gpui::test]
fn two_sections_for_one_context_apply_in_order_and_the_later_wins(cx: &mut TestAppContext) {
    install(
        cx,
        r#"[
          {"context": "Pane", "bindings": {"ctrl-j": "kmtest::Alpha", "ctrl-k": "kmtest::Alpha"}},
          {"context": "Pane", "bindings": {"ctrl-j": "kmtest::Beta"}}
        ]"#,
    );
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "ctrl-j"), ["Beta"]);
    assert_eq!(
        press(cx, window, &log, "ctrl-k"),
        ["Alpha"],
        "the earlier section still applies"
    );
}

#[gpui::test]
fn problems_carry_the_line_of_the_binding_and_the_rest_still_applies(cx: &mut TestAppContext) {
    // Lines: 1 `[`, 2 section, 3 context, 4 bindings, 5 unknown action, 6 bad key, 7 fine,
    // 8-9 closing, 10 second section whose context is invalid.
    let text = "[\n  {\n    \"context\": \"Pane\",\n    \"bindings\": {\n      \"ctrl-x\": \"nope::Missing\",\n      \"not-a-real-modifier-\": \"kmtest::Beta\",\n      \"ctrl-a\": \"kmtest::Alpha\"\n    }\n  },\n  {\"context\": \"Pane &&\", \"bindings\": {\"ctrl-b\": \"kmtest::Beta\"}}\n]";
    install(cx, text);
    let (window, log) = probe(cx, "Pane");
    assert_eq!(
        press(cx, window, &log, "ctrl-a"),
        ["Alpha"],
        "valid bindings apply"
    );

    let found = cx.read(user_diagnostics);
    let located: Vec<_> = found.iter().map(|d| (d.line, d.to_string())).collect();
    assert_eq!(found.len(), 3, "{located:#?}");
    assert_eq!(found[0].line, Some(5));
    assert_eq!(
        found[0].to_string(),
        "keymap.json:5: unknown action `nope::Missing` (binding `ctrl-x`)"
    );
    assert_eq!(found[1].line, Some(6));
    assert!(matches!(
        found[1].problem,
        KeymapProblem::InvalidKeystrokes { .. }
    ));
    assert!(
        found[1].to_string().starts_with("keymap.json:6: "),
        "{}",
        found[1]
    );
    assert_eq!(found[2].line, Some(10));
    assert!(matches!(
        found[2].problem,
        KeymapProblem::InvalidContext { .. }
    ));
}

#[gpui::test]
fn a_syntax_error_reports_the_parsers_line_and_keeps_the_previous_keymap(cx: &mut TestAppContext) {
    install(cx, r#"[{"bindings": {"ctrl-a": "kmtest::Alpha"}}]"#);
    let (window, log) = probe(cx, "Pane");
    cx.update(|cx| {
        reload_user_keymap(
            cx,
            "[\n  {\"bindings\": {\n    \"ctrl-a\": \"kmtest::Beta\"\n",
        )
    });
    assert_eq!(press(cx, window, &log, "ctrl-a"), ["Alpha"]);
    let found = cx.read(user_diagnostics);
    assert!(
        matches!(found[..], [ref d] if d.line == Some(4)),
        "{found:?}"
    );
}

#[gpui::test]
fn one_notification_summarises_every_problem_and_a_fix_clears_it(cx: &mut TestAppContext) {
    install(cx, "[]");
    let (events, _subscription) = listen(cx);
    let bad = "[\n  {\"bindings\": {\n    \"ctrl-x\": \"nope::Missing\",\n    \"not-a-real-modifier-\": \"kmtest::Beta\"\n  }}\n]";

    cx.update(|cx| reload_user_keymap(cx, bad));
    let seen = events.borrow().clone();
    assert_eq!(seen.len(), 1, "one event for the whole file: {seen:?}");
    let message = seen[0].message();
    let mut lines = message.lines();
    assert_eq!(
        lines.next(),
        Some("2 problems in keymap.json; the other bindings were applied.")
    );
    assert_eq!(
        lines.next(),
        Some("keymap.json:3: unknown action `nope::Missing` (binding `ctrl-x`)")
    );
    assert!(
        lines
            .next()
            .is_some_and(|l| l.starts_with("keymap.json:4: ")),
        "{message}"
    );
    assert_eq!(lines.next(), None);

    // The same problems again (an unrelated edit elsewhere in the file) raise nothing.
    let touched = format!("{bad}\n// a comment");
    cx.update(|cx| reload_user_keymap(cx, &touched));
    assert_eq!(events.borrow().len(), 1);

    // Fixing the file raises an empty event so the notification can go away.
    cx.update(|cx| reload_user_keymap(cx, r#"[{"bindings": {"ctrl-x": "kmtest::Alpha"}}]"#));
    let seen = events.borrow().clone();
    assert_eq!(seen.len(), 2);
    assert!(seen[1].is_clear());
}

#[gpui::test]
fn the_embedded_layers_problems_are_not_the_users(cx: &mut TestAppContext) {
    // An action the shipped defaults name that nothing registered is skipped quietly, and no
    // event is raised for a file that is fine.
    let (events, _subscription) = listen(cx);
    install(cx, "");
    assert!(events.borrow().is_empty());
    assert_eq!(cx.read(user_diagnostics), []);
}

#[gpui::test]
fn reload_rereads_the_file_and_the_old_binding_stops_firing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.json");
    std::fs::write(&path, r#"[{"bindings": {"ctrl-j": "kmtest::Alpha"}}]"#).unwrap();
    cx.update(|cx| init_with_dir(dir.path(), mac(), cx));
    assert_eq!(cx.read(user_keymap_file), Some(path.clone()));
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "ctrl-j"), ["Alpha"]);

    std::fs::write(&path, r#"[{"bindings": {"ctrl-k": "kmtest::Beta"}}]"#).unwrap();
    cx.update(reload);
    assert!(
        press(cx, window, &log, "ctrl-j").is_empty(),
        "the old binding is gone"
    );
    assert_eq!(press(cx, window, &log, "ctrl-k"), ["Beta"]);

    // A file that was deleted is an empty keymap, not an error.
    std::fs::remove_file(&path).unwrap();
    cx.update(reload);
    assert!(press(cx, window, &log, "ctrl-k").is_empty());
    assert_eq!(cx.read(user_diagnostics), []);
}

#[gpui::test]
fn a_keymap_installed_from_text_has_no_file(cx: &mut TestAppContext) {
    install(cx, "[]");
    assert_eq!(cx.read(user_keymap_file), None);
    cx.update(reload); // nothing to read, nothing happens
}

#[gpui::test]
fn every_binding_keeps_the_source_it_came_from(cx: &mut TestAppContext) {
    install(cx, r#"[{"bindings": {"ctrl-j": "kmtest::Alpha"}}]"#);
    let layers = |cx: &mut TestAppContext, name: &str| -> Vec<Option<KeybindSource>> {
        cx.read(|cx| bindings_for_action_name(cx, name, None))
            .into_iter()
            .map(|b| b.layer)
            .collect()
    };
    assert_eq!(layers(cx, "kmtest::Alpha"), [Some(KeymapLayer::User)]);
    // `cmd-q` is the default; the user's file did not touch it.
    assert_eq!(layers(cx, "app::Quit"), [Some(KeymapLayer::Default)]);
    // Overriding it puts the user's binding first and it says where it came from.
    cx.update(|cx| reload_user_keymap(cx, r#"[{"bindings": {"cmd-j": "app::Quit"}}]"#));
    assert_eq!(
        layers(cx, "app::Quit"),
        [Some(KeymapLayer::User), Some(KeymapLayer::Default)]
    );
}

#[gpui::test]
fn duplicate_bindings_in_a_context_are_reported_as_conflicts(cx: &mut TestAppContext) {
    install(
        cx,
        "[\n  {\"context\": \"Pane\", \"bindings\": {\"ctrl-j\": \"kmtest::Alpha\"}},\n  {\"context\": \"Pane\", \"bindings\": {\"ctrl-j\": \"kmtest::Beta\"}}\n]",
    );
    let found = cx.read(conflicts);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].layer, KeymapLayer::User);
    assert_eq!(found[0].keystrokes, "ctrl-j");
    assert_eq!(found[0].winner().action, "kmtest::Beta");
    assert_eq!(found[0].winner().line, Some(3));
    // The conflict is information, not an error: nothing is notified.
    assert_eq!(cx.read(user_diagnostics), []);
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "ctrl-j"), ["Beta"]);
}

#[gpui::test]
fn the_shipped_defaults_have_no_duplicates_and_a_user_override_is_not_one(cx: &mut TestAppContext) {
    for platform in [
        KeymapPlatform::MacOs,
        KeymapPlatform::Linux,
        KeymapPlatform::Windows,
    ] {
        let options = KeymapOptions {
            platform,
            vim: true,
        };
        cx.update(|cx| {
            init_with_text(
                r#"[{"bindings": {"ctrl-j": "kmtest::Alpha"}}]"#,
                options,
                cx,
            )
        });
        assert_eq!(cx.read(conflicts), [], "{platform:?}");
    }
}

#[test]
fn a_parsed_file_can_cross_to_the_ui_thread() {
    fn is_send<T: Send>() {}
    is_send::<ParsedUserKeymap>();
}

/// A user file of `count` bindings (two-keystroke sequences, so they are all distinct), a tenth of
/// them naming an action nobody registered.
fn big_file(count: usize) -> String {
    let letter = |n: usize| (b'a' + (n % 26) as u8) as char;
    let mut out = String::from("[\n");
    for section in 0..count.div_ceil(50) {
        out.push_str("  {\"context\": \"Pane\", \"bindings\": {\n");
        for n in (section * 50)..((section + 1) * 50).min(count) {
            let action = match n % 10 {
                0 => "nope::Missing",
                x if x % 2 == 1 => "kmtest::Alpha",
                _ => "kmtest::Beta",
            };
            out.push_str(&format!(
                "    \"ctrl-{} ctrl-{}\": \"{action}\",\n",
                letter(n / 26),
                letter(n)
            ));
        }
        out.push_str("  }},\n");
    }
    out.push(']');
    out
}

#[gpui::test]
fn a_500_binding_file_applies_within_the_reload_budget(cx: &mut TestAppContext) {
    install(cx, "[]");
    let text = big_file(500);

    // Parsing happens on the watcher's thread, off the UI thread: timed on its own.
    let started = Instant::now();
    let parsed = ParsedUserKeymap::parse(&text);
    let parse = started.elapsed();
    drop(parsed);

    // What the UI thread pays: validating every action and swapping the keymap, once.
    let started = Instant::now();
    cx.update(|cx| reload_user_keymap(cx, &text));
    let foreground = started.elapsed();
    eprintln!(
        "500-binding keymap.json: parse {parse:?} (background), apply {foreground:?} (foreground)"
    );

    let found = cx.read(user_diagnostics);
    assert_eq!(found.len(), 50, "one in ten names an unknown action");
    assert!(found.iter().all(|d| d.line.is_some()));
    let (window, log) = probe(cx, "Pane");
    assert_eq!(press(cx, window, &log, "ctrl-a ctrl-b"), ["Alpha"]);
    assert_eq!(press(cx, window, &log, "ctrl-a ctrl-c"), ["Beta"]);
    // A debug build with no optimisation; the release number is a fraction of this. One frame is
    // 8 ms at 120 Hz, and the budget for the swap is a couple of frames.
    assert!(
        foreground < Duration::from_millis(100),
        "apply took {foreground:?}"
    );
    assert!(parse < Duration::from_millis(100), "parse took {parse:?}");
}
