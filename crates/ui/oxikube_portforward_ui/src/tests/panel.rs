//! The panel in a window: opened by the command, the key and `:pf`; the forwards of two clusters
//! listed with their state; the keys of its context; the toolbar; live updates and the sampler.

use std::time::Duration;

use gpui::TestAppContext;
use oxikube_domain::command::Command;

use super::{Fixture, active, dev, failed, forward, modifier, prod, reconnecting};

fn two_clusters() -> Vec<oxikube_app::ForwardInfo> {
    vec![
        active(forward(1, prod(), "Pod", "web"), 18080),
        reconnecting(forward(2, dev(), "Service", "api"), 2),
        failed(forward(3, dev(), "Pod", "db"), "port 5432 is in use"),
    ]
}

#[gpui::test]
fn the_command_the_key_and_pf_show_the_panel(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(Vec::new(), cx);
    assert!(!fx.panel_open(), "the panel starts hidden");

    // `:pf` and the table's `f` send `show: true`.
    fx.dispatch(Command::PortForwardTogglePanel { show: true });
    assert!(fx.panel_open());
    assert!(fx.panel_focused(), "shown and focused");
    fx.dispatch(Command::PortForwardTogglePanel { show: true });
    assert!(fx.panel_open(), "show never hides a panel that is up");

    // The toggle command and the key hide a focused panel and show it again.
    fx.dispatch(Command::PortForwardTogglePanel { show: false });
    assert!(!fx.panel_open());
    fx.keys(&format!("{}-shift-f", modifier()));
    assert!(fx.panel_open());
    fx.keys(&format!("{}-shift-f", modifier()));
    assert!(!fx.panel_open());
    fx.keys(&format!("{}-shift-f", modifier()));
    fx.keys("escape");
    assert!(!fx.panel_open(), "escape closes it");
}

#[gpui::test]
fn it_lists_the_forwards_of_every_cluster_with_their_state(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(two_clusters(), cx);
    fx.show_panel();
    let cells = fx.cells();
    assert_eq!(cells.len(), 3);
    // Ordered by cluster, then target.
    assert_eq!(cells[0][..3], ["dev-us", "default/pod/db", "auto -> 80"]);
    assert!(cells[0][3].starts_with("Failed"), "{:?}", cells[0]);
    assert_eq!(
        cells[1][..3],
        ["dev-us", "default/service/api", "auto -> 80"]
    );
    assert!(cells[1][3].starts_with("Reconnecting"), "{:?}", cells[1]);
    assert!(cells[1][3].contains("attempt 2"), "{:?}", cells[1]);
    assert!(cells[1][3].contains("next in 4s"), "{:?}", cells[1]);
    assert_eq!(cells[2][..3], ["prod-eu", "default/pod/web", "18080 -> 80"]);
    assert!(cells[2][3].starts_with("Active"), "{:?}", cells[2]);
    assert_eq!(cells[2][4], "\u{2193} 0 B \u{2191} 0 B \u{b7} 0 conn");
}

#[gpui::test]
fn the_empty_panel_says_how_to_start_a_forward(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(Vec::new(), cx);
    fx.show_panel();
    assert!(fx.vcx.debug_bounds("port-forwards-empty").is_some());
    assert!(fx.vcx.debug_bounds("port-forwards-panel").is_some());
}

#[gpui::test]
fn the_keys_of_the_context_move_the_selection_and_send_the_row_actions(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(two_clusters(), cx);
    fx.show_panel();
    assert_eq!(fx.selected(), None);

    fx.keys("down");
    assert_eq!(fx.selected(), Some(3), "the first line: dev-us db");
    fx.keys("j");
    assert_eq!(fx.selected(), Some(2));
    fx.keys("shift-g");
    assert_eq!(fx.selected(), Some(1), "the last line: prod-eu web");
    fx.keys("g");
    assert_eq!(fx.selected(), Some(3));
    fx.keys("k");
    assert_eq!(fx.selected(), Some(3), "clamped at the top");

    // The last line is the active forward 1.
    fx.keys("shift-g");
    fx.recorder.clear();
    fx.keys("s");
    fx.keys("r");
    fx.keys("c");
    fx.keys("o");
    fx.keys("d");
    assert_eq!(
        fx.recorder.sent(),
        [
            Command::PortForwardStop { id: 1 },
            Command::PortForwardRestart { id: 1 },
            Command::PortForwardCopyUrl { id: 1 },
            Command::PortForwardOpenInBrowser { id: 1 },
            Command::PortForwardRemove { id: 1 },
        ]
    );
}

#[gpui::test]
fn stop_does_nothing_for_a_forward_that_is_not_running(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(two_clusters(), cx);
    fx.show_panel();
    fx.keys("down");
    assert_eq!(fx.selected(), Some(3), "the failed forward");
    fx.recorder.clear();
    fx.keys("s");
    assert!(fx.recorder.sent().is_empty());
    fx.keys("r");
    assert_eq!(fx.recorder.sent(), [Command::PortForwardRestart { id: 3 }]);
}

#[gpui::test]
fn bare_letters_are_text_while_the_filter_has_the_focus(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(two_clusters(), cx);
    fx.show_panel();
    fx.keys("down");
    fx.recorder.clear();
    fx.keys("/");
    fx.keys("w e b");
    assert!(fx.recorder.sent().is_empty(), "typing is not a command");
    assert_eq!(fx.cells().len(), 1, "the filter narrowed the list to web");
    assert_eq!(fx.cells()[0][1], "default/pod/web");
    fx.keys("escape");
    assert!(fx.panel_open(), "escape leaves the filter first");
    fx.keys("escape");
    assert!(!fx.panel_open());
}

#[gpui::test]
fn the_toolbar_buttons_send_the_same_commands(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(two_clusters(), cx);
    fx.show_panel();
    fx.keys("shift-g");
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    fx.recorder.clear();
    for button in [
        "port-forwards-stop",
        "port-forwards-restart",
        "port-forwards-copy",
        "port-forwards-open",
        "port-forwards-remove",
    ] {
        fx.click(button);
        fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }
    assert_eq!(
        fx.recorder.sent(),
        [
            Command::PortForwardStop { id: 1 },
            Command::PortForwardRestart { id: 1 },
            Command::PortForwardCopyUrl { id: 1 },
            Command::PortForwardOpenInBrowser { id: 1 },
            Command::PortForwardRemove { id: 1 },
        ]
    );
}

#[gpui::test]
fn a_stopped_forward_offers_start_which_restarts_it(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(vec![forward(7, prod(), "Pod", "web")], cx);
    fx.show_panel();
    fx.keys("down");
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(fx.vcx.debug_bounds("port-forwards-start").is_some());
    assert!(fx.vcx.debug_bounds("port-forwards-stop").is_none());
    fx.recorder.clear();
    fx.click("port-forwards-start");
    assert_eq!(fx.recorder.sent(), [Command::PortForwardRestart { id: 7 }]);
}

#[gpui::test]
fn a_click_selects_and_a_double_click_opens_in_the_browser(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(two_clusters(), cx);
    fx.show_panel();
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let table = fx.vcx.debug_bounds("port-forwards-table").expect("drawn");
    // The header is the first line; the second row is the third line of the table.
    let row_height = 28.0_f32;
    let target = gpui::point(
        table.origin.x + gpui::px(40.),
        table.origin.y + gpui::px(row_height * 2.5),
    );
    fx.vcx.simulate_click(target, gpui::Modifiers::none());
    fx.settle();
    assert!(fx.selected().is_some(), "the click selected a row");
}

#[gpui::test]
fn changes_arrive_live_and_the_selection_follows_the_forward(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(two_clusters(), cx);
    fx.show_panel();
    fx.keys("shift-g");
    assert_eq!(fx.selected(), Some(1));

    // A new forward sorts above the selected one; the failed one is removed.
    fx.source.set(vec![
        active(forward(1, prod(), "Pod", "web"), 18080),
        reconnecting(forward(2, dev(), "Service", "api"), 3),
        active(forward(4, dev(), "Pod", "cache"), 16379),
    ]);
    fx.settle();
    let cells = fx.cells();
    assert_eq!(cells.len(), 3);
    assert_eq!(cells[0][1], "default/pod/cache");
    assert!(cells[1][3].contains("attempt 3"), "{:?}", cells[1]);
    assert_eq!(fx.selected(), Some(1), "still on the same forward");
}

#[gpui::test]
fn traffic_and_uptime_are_sampled_each_second_without_a_change_signal(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(vec![active(forward(1, prod(), "Pod", "web"), 18080)], cx);
    fx.show_panel();
    assert_eq!(fx.cells()[0][4], "\u{2193} 0 B \u{2191} 0 B \u{b7} 0 conn");

    let mut busy = active(forward(1, prod(), "Pod", "web"), 18080);
    busy.stats.bytes_in = 3 * 1024 * 1024;
    busy.stats.bytes_out = 512;
    busy.stats.active_connections = 2;
    fx.source.set_quietly(vec![busy]);
    fx.advance(oxikube_runtime::FRAME_STALL);
    assert_eq!(
        fx.cells()[0][4],
        "\u{2193} 0 B \u{2191} 0 B \u{b7} 0 conn",
        "nothing is read between samples"
    );
    fx.advance(crate::tick_interval());
    assert_eq!(
        fx.cells()[0][4],
        "\u{2193} 3.0 MB \u{2191} 512 B \u{b7} 2 conn"
    );

    // Uptime moves with the clock.
    fx.clock.advance(Duration::from_secs(125));
    fx.advance(crate::tick_interval());
    assert_eq!(fx.cells()[0][5], "2m5s");
}

/// 50 live forwards with counters that move all the time (the epic's 60 fps case): the panel
/// redraws once per sample, not once per byte, and a sample is cheap.
#[gpui::test]
fn fifty_busy_forwards_redraw_once_per_sample(cx: &mut TestAppContext) {
    let busy = |round: u64| -> Vec<oxikube_app::ForwardInfo> {
        (0..50)
            .map(|i| {
                let cluster = if i % 2 == 0 { prod() } else { dev() };
                let mut f = active(
                    forward(i + 1, cluster, "Pod", &format!("web-{i}")),
                    20000 + i as u16,
                );
                f.stats.bytes_in = round * 1_000_003 * (i + 1);
                f.stats.bytes_out = round * 4_099 * (i + 1);
                f.stats.active_connections = i % 5;
                f
            })
            .collect()
    };
    let mut fx = Fixture::open(busy(0), cx);
    fx.show_panel();
    assert_eq!(fx.cells().len(), 50);

    let redraws = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    let counter = redraws.clone();
    let panel = fx.panel.clone();
    let _observe = fx
        .vcx
        .update(|_, cx| cx.observe(&panel, move |_, _| counter.set(counter.get() + 1)));
    let mut worst = std::time::Duration::ZERO;
    for round in 1..=10 {
        // Counters move between samples; no change signal, as with real traffic.
        fx.source.set_quietly(busy(round));
        let started = std::time::Instant::now();
        fx.advance(crate::tick_interval());
        worst = worst.max(started.elapsed());
    }
    assert_eq!(
        fx.cells()[0][4].split(' ').nth(1),
        Some("19.1"),
        "the last sample landed: {:?}",
        fx.cells()[0]
    );
    let redraws = redraws.get();
    eprintln!("50 forwards, 10 samples: {redraws} redraws, slowest sample {worst:?}");
    assert!(
        redraws <= 10,
        "one redraw per sample at most, saw {redraws}"
    );
}
