//! The `ClusterTab` key context (E11-S07): set by the tab, above everything in it, so one
//! binding (`:` opens the jump bar, `?` the help overlay) reaches the table, the drawer, the logs
//! and the terminal of a cluster, and no other screen.

use gpui::{Focusable as _, KeyContext, TestAppContext};
use oxikube_keymap::KeyContextual as _;

use super::*;

/// The key contexts from the window root to the focused element, outermost first.
fn stack(fx: &mut Fixture) -> Vec<KeyContext> {
    fx.vcx.update(|window, cx| {
        window.draw(cx).clear(cx);
        window.context_stack()
    })
}

fn names(stack: &[KeyContext], context: &str) -> usize {
    stack.iter().filter(|c| c.contains(context)).count()
}

#[gpui::test]
fn a_focused_element_in_a_cluster_tab_has_the_cluster_tab_context_above_it(
    cx: &mut TestAppContext,
) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    fx.connect("alpha");
    let inner = fx.inner("alpha");
    fx.vcx.update(|window, cx| {
        let item = TestItem::build("pods", cx);
        inner.update(cx, |ws, cx| ws.open_item(item.clone(), window, cx));
        window.focus(&item.focus_handle(cx), cx);
    });
    fx.vcx.run_until_parked();

    let stack = stack(&mut fx);
    assert_eq!(names(&stack, "ClusterTab"), 1, "{stack:?}");
    let tab = stack
        .iter()
        .position(|c| c.contains("ClusterTab"))
        .expect("the tab's context");
    assert!(
        stack[tab].contains("connected"),
        "a connected session says so: {:?}",
        stack[tab]
    );
    // The tab's context sits above the cluster's own workspace, so one binding in it reaches
    // everything in the tab.
    let inner_workspace = stack
        .iter()
        .rposition(|c| c.contains("Workspace"))
        .expect("the cluster's workspace");
    assert!(tab < inner_workspace, "{stack:?}");
}

#[gpui::test]
fn a_screen_outside_the_cluster_tabs_has_no_cluster_tab_context(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    fx.connect("alpha");
    // The window's own first tab (the catalog home's stand-in), not a cluster's.
    let home = fx.vcx.update(|_, cx| {
        fx.ws
            .read(cx)
            .items()
            .find(|item| item.tab_content(cx).title == "Clusters")
            .map(|item| item.item_id())
            .expect("the Clusters tab")
    });
    fx.vcx.update(|window, cx| {
        fx.ws
            .update(cx, |ws, cx| ws.activate_item(home, true, window, cx));
    });
    fx.vcx.run_until_parked();
    let stack = stack(&mut fx);
    assert_eq!(names(&stack, "ClusterTab"), 0, "{stack:?}");
}

#[gpui::test]
fn the_context_says_connected_only_while_the_session_is_up(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    fx.connect("alpha");
    let tab = fx.tab("alpha");
    assert!(
        fx.vcx
            .update(|_, cx| tab.read(cx).key_context().contains("connected"))
    );
    fx.vcx.update(|_, cx| {
        tab.update(cx, |tab, cx| {
            let mut info = tab.info().clone();
            info.state = oxikube_domain::session::ClusterSessionState::default();
            tab.set_info(info, cx);
        });
    });
    assert!(
        !fx.vcx
            .update(|_, cx| tab.read(cx).key_context().contains("connected"))
    );
    assert!(
        fx.vcx
            .update(|_, cx| tab.read(cx).key_context().contains("ClusterTab"))
    );
}
