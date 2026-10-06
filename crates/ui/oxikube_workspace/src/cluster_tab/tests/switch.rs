//! `cmd-1..9`, next and previous, `cluster::Select`: what the keys and commands do.

use gpui::TestAppContext;

use super::*;

fn three(cx: &mut TestAppContext) -> Fixture {
    let mut fx = Fixture::open(cx, &["alpha", "beta", "gamma"]);
    for name in ["alpha", "beta", "gamma"] {
        fx.connect(name);
    }
    fx
}

fn key(fx: &mut Fixture, number: u8) {
    fx.vcx
        .simulate_keystrokes(&format!("{}-{number}", modifier()));
    fx.vcx.run_until_parked();
}

#[gpui::test]
fn cmd_n_shows_the_nth_cluster_tab(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    assert_eq!(
        fx.active_name().as_deref(),
        Some("gamma"),
        "the last opened is shown"
    );

    key(&mut fx, 2);
    assert_eq!(fx.active_name().as_deref(), Some("beta"));
    key(&mut fx, 1);
    assert_eq!(fx.active_name().as_deref(), Some("alpha"));
    key(&mut fx, 3);
    assert_eq!(fx.active_name().as_deref(), Some("gamma"));

    // There is no fourth tab: the key does nothing.
    key(&mut fx, 4);
    key(&mut fx, 9);
    assert_eq!(fx.active_name().as_deref(), Some("gamma"));
}

#[gpui::test]
fn the_shipped_keymap_binds_one_to_nine_and_the_cycling_keys(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    // `ctrl-tab` and `ctrl-shift-tab` are bound on every OS.
    fx.vcx.simulate_keystrokes("ctrl-shift-tab");
    fx.vcx.run_until_parked();
    assert_eq!(fx.active_name().as_deref(), Some("beta"));
    fx.vcx.simulate_keystrokes("ctrl-tab");
    fx.vcx.simulate_keystrokes("ctrl-tab");
    fx.vcx.run_until_parked();
    assert_eq!(fx.active_name().as_deref(), Some("alpha"), "wraps around");
}

#[gpui::test]
fn next_and_previous_wrap_around(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    assert!(fx.apply(Command::ClusterNextTab));
    assert_eq!(
        fx.active_name().as_deref(),
        Some("alpha"),
        "after the last comes the first"
    );
    assert!(fx.apply(Command::ClusterPreviousTab));
    assert_eq!(
        fx.active_name().as_deref(),
        Some("gamma"),
        "before the first comes the last"
    );
    assert!(fx.apply(Command::ClusterPreviousTab));
    assert_eq!(fx.active_name().as_deref(), Some("beta"));
}

#[gpui::test]
fn next_from_the_catalog_shows_the_first_and_previous_the_last(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    let home = fx.vcx.update(|_, cx| {
        fx.ws
            .read(cx)
            .items()
            .find(|item| item.tab_content(cx).title == "Clusters")
            .map(|item| item.item_id())
            .expect("the catalog tab")
    });
    let show_home = |fx: &mut Fixture| {
        fx.vcx.update(|window, cx| {
            fx.ws
                .update(cx, |ws, cx| ws.activate_item(home, true, window, cx))
        });
        fx.vcx.run_until_parked();
    };
    show_home(&mut fx);
    assert_eq!(fx.active_name(), None);
    assert!(fx.apply(Command::ClusterNextTab));
    assert_eq!(fx.active_name().as_deref(), Some("alpha"));
    show_home(&mut fx);
    assert!(fx.apply(Command::ClusterPreviousTab));
    assert_eq!(fx.active_name().as_deref(), Some("gamma"));
}

#[gpui::test]
fn select_shows_the_cluster_and_ignores_one_without_a_tab(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    assert!(fx.apply(Command::ClusterSelect {
        cluster: id("alpha")
    }));
    assert_eq!(fx.active_name().as_deref(), Some("alpha"));
    assert!(!fx.apply(Command::ClusterSelect {
        cluster: id("unknown")
    }));
    assert_eq!(fx.active_name().as_deref(), Some("alpha"));
    assert!(
        !fx.apply(Command::ClusterSwitchTab { index: 0 }),
        "tabs count from 1"
    );
    assert!(!fx.apply(Command::PaletteToggle), "not a tab command");
}

#[gpui::test]
fn the_nth_tab_follows_the_order_the_tabs_are_shown_in(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    // Drag gamma to the front of its pane.
    let (gamma, pane) = (
        fx.tab("gamma").entity_id(),
        fx.vcx
            .update(|_, cx| fx.ws.read(cx).active_pane(cx))
            .expect("a pane")
            .id(),
    );
    // Index 0 of the pane is the catalog tab; put gamma right after it.
    fx.vcx.update(|window, cx| {
        fx.ws
            .update(cx, |ws, cx| ws.move_item(gamma, pane, Some(1), window, cx))
    });
    fx.vcx.run_until_parked();
    assert_eq!(fx.open_names(), ["gamma", "alpha", "beta"]);
    key(&mut fx, 1);
    assert_eq!(fx.active_name().as_deref(), Some("gamma"));
    key(&mut fx, 3);
    assert_eq!(fx.active_name().as_deref(), Some("beta"));
}

#[gpui::test]
fn switching_does_nothing_in_a_window_without_cluster_tabs(cx: &mut TestAppContext) {
    // The key is handled at the app level: another window must not be affected by this one.
    let mut fx = three(cx);
    let (other, mut other_vcx) = open_workspace(cx);
    other_vcx.update(|window, cx| {
        let item = TestItem::build("Clusters", cx);
        other.update(cx, |ws, cx| ws.open_item(item, window, cx));
    });
    other_vcx.simulate_keystrokes(&format!("{}-1", modifier()));
    other_vcx.run_until_parked();
    fx.vcx.run_until_parked();
    assert_eq!(fx.active_name().as_deref(), Some("gamma"));
}
