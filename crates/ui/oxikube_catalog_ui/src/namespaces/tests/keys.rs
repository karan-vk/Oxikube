//! The keyboard: `0`-`9`, movement, pinning, escape.

use gpui::TestAppContext;
use oxikube_domain::session::NamespaceSelection;

use super::*;

/// Focuses the closed trigger, as tabbing to it would.
fn focus_trigger(window: &mut TestWindow<NamespaceSelector>) {
    window.update_root(|selector, window, cx| window.focus(&selector.trigger_focus.clone(), cx));
}

fn single(name: &str) -> NamespaceSelection {
    NamespaceSelection::single(name)
}

#[gpui::test]
fn digit_keys_select_the_favourites_and_zero_selects_all(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod", "stage"]);
    env.remember(&prefs(&[], &["prod", "dev", "stage"]));
    let mut window = open(cx, &env);
    window.run_until_parked();
    focus_trigger(&mut window);

    window.simulate_keystrokes("2");
    window.run_until_parked();
    assert_eq!(
        env.session_selection(),
        single("dev"),
        "2 is the second favourite"
    );
    assert_eq!(window.read_root(|s, _| s.label()), "dev");

    window.simulate_keystrokes("1");
    window.run_until_parked();
    assert_eq!(env.session_selection(), single("prod"));

    window.simulate_keystrokes("0");
    window.run_until_parked();
    assert_eq!(
        env.session_selection(),
        NamespaceSelection::All,
        "0 is all namespaces"
    );

    window.simulate_keystrokes("3");
    window.run_until_parked();
    assert_eq!(env.session_selection(), single("stage"));
    assert_eq!(
        env.stored().selection,
        single("stage"),
        "digits are remembered too"
    );
}

#[gpui::test]
fn a_digit_with_no_favourite_does_nothing(cx: &mut TestAppContext) {
    let env = Env::new(&["dev"]);
    env.remember(&prefs(&["dev"], &["dev"]));
    let mut window = open(cx, &env);
    window.run_until_parked();
    focus_trigger(&mut window);

    window.simulate_keystrokes("5 9");
    window.run_until_parked();

    assert_eq!(env.session_selection(), single("dev"));
}

#[gpui::test]
fn digits_are_text_in_the_search_box_and_keys_in_the_list(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod"]);
    env.remember(&prefs(&[], &["prod"]));
    let mut window = open(cx, &env);
    window.run_until_parked();
    focus_trigger(&mut window);
    window.simulate_keystrokes("enter");
    window.run_until_parked();
    assert!(
        window.read_root(|s, _| s.is_open()),
        "enter on the trigger opens"
    );

    // The search box has focus: the digit is typed, nothing is selected.
    window.simulate_keystrokes("1");
    window.run_until_parked();
    assert_eq!(window.read_root(|s, _| s.query.to_string()), "1");
    assert_eq!(env.session_selection(), NamespaceSelection::All);

    // From the list the same key selects the favourite.
    window.update_root(|s, window, cx| window.focus(&s.list_focus.clone(), cx));
    window.simulate_keystrokes("1");
    window.run_until_parked();
    assert_eq!(env.session_selection(), single("prod"));
}

#[gpui::test]
fn arrows_and_enter_tick_namespaces_and_f_pins(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod", "stage"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    focus_trigger(&mut window);
    window.simulate_keystrokes("enter");
    window.run_until_parked();

    // Down leaves the search box for the list. Rows: All (highlighted), # Namespaces, dev,
    // prod, stage; the header is skipped.
    window.simulate_keystrokes("down");
    window.run_until_parked();
    window.simulate_keystrokes("down down enter");
    window.run_until_parked();
    window.simulate_keystrokes("down space");
    env.settle(&window);
    assert_eq!(
        env.session_selection(),
        NamespaceSelection::from_names(["prod", "stage"])
    );

    window.simulate_keystrokes("f");
    window.run_until_parked();
    assert_eq!(
        window.read_root(|s, _| s.favourites().iter().map(str::to_owned).collect::<Vec<_>>()),
        ["stage"],
        "f pins the highlighted namespace"
    );
    assert_eq!(
        env.stored().favourites.iter().collect::<Vec<_>>(),
        ["stage"]
    );
}

#[gpui::test]
fn escape_closes_and_returns_focus_to_the_trigger(cx: &mut TestAppContext) {
    let env = Env::new(&["dev"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);

    window.simulate_keystrokes("escape");
    window.run_until_parked();

    assert!(!window.read_root(|s, _| s.is_open()));
    let focused = window.update_root(|s, window, _| s.trigger_focus.is_focused(window));
    assert!(focused, "focus is back on the trigger");

    // From the list as well.
    open_dropdown(&mut window);
    window.update_root(|s, window, cx| window.focus(&s.list_focus.clone(), cx));
    window.simulate_keystrokes("escape");
    window.run_until_parked();
    assert!(!window.read_root(|s, _| s.is_open()));
}

#[gpui::test]
fn the_search_enter_ticks_the_first_match(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod", "stage"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);

    window.simulate_input("sta");
    window.simulate_keystrokes("enter");
    env.settle(&window);

    assert_eq!(env.session_selection(), single("stage"));
}
