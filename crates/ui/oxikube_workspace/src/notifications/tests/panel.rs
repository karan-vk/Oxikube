//! The panel: toggled by the command, the key and the bell; the unread badge; opening a row
//! navigates; the keys of its context; the mute toggle.

use gpui::TestAppContext;
use oxikube_domain::command::Command;

use super::{Fixture, modifier, pod, prod, warning};

#[gpui::test]
fn the_command_and_the_key_toggle_the_panel(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    assert!(!fx.panel_open(), "the panel starts hidden");

    fx.dispatch(Command::NotificationsTogglePanel);
    assert!(fx.panel_open());
    assert!(fx.panel_focused(), "shown and focused");

    // Shown and focused: the key hides it.
    fx.keys(&format!("{}-shift-i", modifier()));
    assert!(!fx.panel_open());
    fx.keys(&format!("{}-shift-i", modifier()));
    assert!(fx.panel_open());
    assert!(fx.panel_focused());
    fx.keys("escape");
    assert!(!fx.panel_open(), "escape closes it");
}

#[gpui::test]
fn the_bell_shows_the_unread_count_and_toggles_the_panel(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    assert_eq!(fx.badge(), 0);
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(fx.vcx.debug_bounds("status-notifications").is_some());
    assert!(fx.vcx.debug_bounds("status-notifications-count").is_none());

    fx.submit(warning("web-0", "BackOff"));
    fx.submit(warning("web-1", "BackOff"));
    assert_eq!(fx.badge(), 2);
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(fx.vcx.debug_bounds("status-notifications-count").is_some());

    fx.click("status-notifications");
    assert!(
        fx.recorder
            .sent()
            .contains(&Command::NotificationsTogglePanel)
    );
    assert!(fx.panel_open());

    fx.dispatch(Command::NotificationsMarkAllRead);
    assert_eq!(fx.badge(), 0);
}

#[gpui::test]
fn a_click_on_a_row_marks_it_read_and_opens_its_object(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    fx.submit(warning("web-0", "BackOff"));
    fx.submit(warning("web-1", "Failed"));
    fx.show_panel();
    fx.recorder.clear();

    // Newest first: row 1 is web-0.
    fx.click("notification-row-1");
    let id = fx.service.list()[1].id;
    assert_eq!(
        fx.recorder.sent(),
        [
            Command::NotificationsMarkRead { id: id.0 },
            Command::ClusterSelect { cluster: prod() },
            Command::ResourceOpen {
                target: pod("web-0")
            },
        ]
    );
    assert!(fx.service.get(id).unwrap().read);
    assert_eq!(fx.badge(), 1);
}

#[gpui::test]
fn the_panel_keys_select_open_and_mark_read(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    for name in ["a", "b", "c"] {
        fx.submit(warning(name, "BackOff"));
    }
    fx.show_panel();
    let panel = fx.wiring.panel.clone();
    let selected = |fx: &mut Fixture| fx.vcx.update(|_, cx| panel.read(cx).selected());
    assert_eq!(selected(&mut fx), 0);
    fx.keys("down");
    fx.keys("j");
    assert_eq!(selected(&mut fx), 2);
    fx.keys("down");
    assert_eq!(selected(&mut fx), 2, "stays on the last row");
    fx.keys("up");
    assert_eq!(selected(&mut fx), 1);

    fx.recorder.clear();
    fx.keys("r");
    let b = fx.service.list()[1].id;
    assert_eq!(
        fx.recorder.sent(),
        [Command::NotificationsMarkRead { id: b.0 }]
    );
    assert_eq!(fx.badge(), 2);

    fx.recorder.clear();
    fx.keys("enter");
    assert_eq!(
        fx.recorder.sent()[1..],
        [
            Command::ClusterSelect { cluster: prod() },
            Command::ResourceOpen { target: pod("b") },
        ]
    );

    fx.keys("shift-r");
    assert!(
        fx.recorder
            .sent()
            .contains(&Command::NotificationsMarkAllRead)
    );
    assert_eq!(fx.badge(), 0);
}

#[gpui::test]
fn the_header_mutes_and_unmutes_the_shown_cluster(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    fx.show_panel();
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        fx.vcx.debug_bounds("notification-mute").is_none(),
        "no cluster tab shown: nothing to mute"
    );
    let panel = fx.wiring.panel.clone();
    fx.vcx
        .update(|_, cx| panel.update(cx, |panel, cx| panel.set_active_cluster(Some(prod()), cx)));
    fx.settle();
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));

    fx.click("notification-mute");
    assert_eq!(
        fx.recorder.sent().last(),
        Some(&Command::NotificationsMuteCluster { cluster: prod() })
    );
    assert!(fx.service.is_muted(&prod()));
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(fx.vcx.debug_bounds("notification-muted-count").is_some());

    fx.submit(warning("web-9", "BackOff"));
    assert!(fx.service.list().is_empty(), "muted: nothing listed");

    fx.click("notification-mute");
    assert_eq!(
        fx.recorder.sent().last(),
        Some(&Command::NotificationsUnmuteCluster { cluster: prod() })
    );
    assert!(!fx.service.is_muted(&prod()));
}

#[gpui::test]
fn an_empty_centre_says_so_and_rows_are_virtualised(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    fx.show_panel();
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(fx.vcx.debug_bounds("notification-empty").is_some());

    for i in 0..300 {
        fx.service.submit(warning(&format!("pod-{i}"), "BackOff"));
    }
    fx.settle();
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(fx.vcx.debug_bounds("notification-row-0").is_some());
    assert!(
        fx.vcx.debug_bounds("notification-row-299").is_none(),
        "only the visible rows are built"
    );
}

#[gpui::test]
fn the_selection_follows_its_row_when_a_newer_one_arrives(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx);
    fx.submit(warning("a", "BackOff"));
    fx.submit(warning("b", "BackOff"));
    fx.show_panel();
    fx.keys("down");
    let panel = fx.wiring.panel.clone();
    let selected = |fx: &mut Fixture| fx.vcx.update(|_, cx| panel.read(cx).selected());
    assert_eq!(selected(&mut fx), 1, "on a");
    fx.submit(warning("c", "BackOff"));
    assert_eq!(selected(&mut fx), 2, "still on a, pushed down by c");
    fx.recorder.clear();
    fx.keys("enter");
    assert_eq!(
        fx.recorder.sent().last(),
        Some(&Command::ResourceOpen { target: pod("a") })
    );
}
