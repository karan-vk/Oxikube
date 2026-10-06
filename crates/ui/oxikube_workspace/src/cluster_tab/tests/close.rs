//! Closing a tab: disconnect, and the confirmation while operations run.

use gpui::{Bounds, Pixels, Point, TestAppContext, point};
use oxikube_domain::session::SessionPhase;

use super::*;
use crate::{
    DialogModal,
    session::{RunningOperation, register_operation_provider},
};

fn center(bounds: Bounds<Pixels>) -> Point<Pixels> {
    point(
        bounds.origin.x + bounds.size.width / 2.,
        bounds.origin.y + bounds.size.height / 2.,
    )
}

/// Clicks the close button of `name`'s tab.
fn click_close(fx: &mut Fixture, name: &str) {
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let selector: &'static str = Box::leak(format!("tab-close-{name}").into_boxed_str());
    let bounds = fx
        .vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{name}'s tab has a close button"));
    fx.vcx.simulate_click(center(bounds), Default::default());
    fx.vcx.run_until_parked();
}

fn click(fx: &mut Fixture, selector: &'static str) {
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = fx
        .vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is on screen"));
    fx.vcx.simulate_click(center(bounds), Default::default());
    fx.vcx.run_until_parked();
}

fn phase(fx: &Fixture, name: &str) -> Option<SessionPhase> {
    fx.sessions.get(&id(name)).map(|s| s.phase())
}

fn dialog_open(fx: &mut Fixture) -> bool {
    fx.vcx.update(|_, cx| {
        fx.ws
            .read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<DialogModal>()
            .is_some()
    })
}

/// An exec session on `name` that is running while the returned flag is true.
fn running_exec(fx: &mut Fixture, name: &str) -> Rc<std::cell::Cell<bool>> {
    let running = Rc::new(std::cell::Cell::new(true));
    let flag = running.clone();
    let cluster = id(name);
    fx.vcx.update(|_, cx| {
        register_operation_provider(cx, move |_| {
            if flag.get() {
                vec![
                    RunningOperation::new("Exec session", "pod/web-0 in prod")
                        .on_cluster(cluster.clone()),
                ]
            } else {
                Vec::new()
            }
        });
    });
    running
}

#[gpui::test]
fn closing_a_tab_disconnects_the_session(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    assert_eq!(phase(&fx, "alpha"), Some(SessionPhase::Ready));

    click_close(&mut fx, "alpha");

    // Nothing was running: no question, `cluster::Disconnect` goes out, the session
    // disconnects and its tab goes with it. The other cluster is untouched.
    assert_eq!(
        fx.recorder.sent(),
        [Command::ClusterDisconnect {
            cluster: id("alpha")
        }]
    );
    assert!(!dialog_open(&mut fx));
    assert_eq!(phase(&fx, "alpha"), Some(SessionPhase::Disconnected));
    assert_eq!(phase(&fx, "beta"), Some(SessionPhase::Ready));
    assert_eq!(fx.open_names(), ["beta"]);
}

#[gpui::test]
fn closing_a_tab_with_a_running_operation_asks_and_cancel_keeps_it(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    running_exec(&mut fx, "alpha");

    click_close(&mut fx, "alpha");
    assert!(
        dialog_open(&mut fx),
        "the confirmation is on the modal layer"
    );
    assert_eq!(
        fx.recorder.disconnects(),
        0,
        "nothing is closed before the answer"
    );
    // It names the cluster and lists what would be stopped.
    let (title, message) = fx.vcx.update(|_, cx| {
        let dialog = fx
            .ws
            .read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<DialogModal>()
            .expect("dialog");
        let dialog = dialog.read(cx);
        (
            dialog.title().to_string(),
            dialog.message_text().map(|m| m.to_string()),
        )
    });
    assert_eq!(title, "Close alpha?");
    let message = message.expect("a message");
    assert!(
        message.contains("Exec session: pod/web-0 in prod"),
        "{message}"
    );

    click(&mut fx, "dialog-cancel");
    assert!(!dialog_open(&mut fx));
    assert_eq!(fx.recorder.disconnects(), 0);
    assert_eq!(phase(&fx, "alpha"), Some(SessionPhase::Ready));
    assert_eq!(fx.open_names(), ["alpha", "beta"], "the tab stayed");
}

#[gpui::test]
fn confirming_the_prompt_disconnects(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    running_exec(&mut fx, "beta");

    click_close(&mut fx, "beta");
    assert!(dialog_open(&mut fx));
    click(&mut fx, "dialog-confirm");

    assert_eq!(
        fx.recorder.sent(),
        [Command::ClusterDisconnect {
            cluster: id("beta")
        }]
    );
    assert_eq!(phase(&fx, "beta"), Some(SessionPhase::Disconnected));
    assert_eq!(fx.open_names(), ["alpha"]);
}

#[gpui::test]
fn operations_of_another_cluster_do_not_block_closing(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    running_exec(&mut fx, "beta");

    click_close(&mut fx, "alpha");
    assert!(
        !dialog_open(&mut fx),
        "beta's exec session says nothing about alpha"
    );
    assert_eq!(fx.open_names(), ["beta"]);
}

#[gpui::test]
fn escape_on_the_prompt_keeps_the_tab(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    fx.connect("alpha");
    running_exec(&mut fx, "alpha");
    click_close(&mut fx, "alpha");
    assert!(dialog_open(&mut fx));
    fx.vcx.simulate_keystrokes("escape");
    fx.vcx.run_until_parked();
    assert!(!dialog_open(&mut fx));
    assert_eq!(fx.recorder.disconnects(), 0);
    assert_eq!(fx.open_names(), ["alpha"]);
}

#[gpui::test]
fn the_close_command_asks_like_the_button(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    let running = running_exec(&mut fx, "alpha");

    assert!(fx.apply(Command::ClusterCloseTab {
        cluster: id("alpha")
    }));
    assert!(dialog_open(&mut fx));
    assert_eq!(fx.recorder.disconnects(), 0);
    fx.vcx.simulate_keystrokes("escape");
    fx.vcx.run_until_parked();

    running.set(false);
    assert!(fx.apply(Command::ClusterCloseTab {
        cluster: id("alpha")
    }));
    assert_eq!(fx.open_names(), ["beta"]);
    assert!(
        !fx.apply(Command::ClusterCloseTab {
            cluster: id("alpha")
        }),
        "already closed"
    );
}

#[gpui::test]
fn close_item_key_closes_the_cluster_items_first_then_the_tab(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    let inner = fx.inner("beta");
    fx.vcx.update(|window, cx| {
        let item = TestItem::build("pods", cx);
        inner.update(cx, |ws, cx| ws.open_item(item, window, cx));
    });
    fx.vcx.run_until_parked();
    let close = format!("{}-w", modifier());

    // The first press closes the pods tab inside beta ...
    fx.vcx.simulate_keystrokes(&close);
    fx.vcx.run_until_parked();
    assert_eq!(fx.vcx.update(|_, cx| inner.read(cx).items().count()), 0);
    assert_eq!(fx.open_names(), ["alpha", "beta"]);
    assert_eq!(fx.recorder.disconnects(), 0);

    // ... and with nothing left in the cluster, the next one closes beta's own tab.
    fx.vcx.simulate_keystrokes(&close);
    fx.vcx.run_until_parked();
    assert_eq!(
        fx.recorder.sent(),
        [Command::ClusterDisconnect {
            cluster: id("beta")
        }]
    );
    assert_eq!(fx.open_names(), ["alpha"]);
}

#[gpui::test]
fn only_cluster_tabs_intercept_the_close(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    fx.connect("alpha");
    let tab = fx.tab("alpha");
    assert!(
        fx.vcx
            .update(|_, cx| crate::Item::intercepts_close(tab.read(cx), cx))
    );
    let catalog_intercepts = fx.vcx.update(|_, cx| {
        fx.ws
            .read(cx)
            .items()
            .find(|item| item.tab_content(cx).title == "Clusters")
            .map(|item| item.intercepts_close(cx))
            .expect("catalog tab")
    });
    assert!(
        !catalog_intercepts,
        "the catalog tab closes as any item does"
    );
}
