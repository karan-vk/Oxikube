//! The order of the tiles: dragging, saving and restoring it.

use gpui::{Modifiers, MouseButton, TestAppContext, px};
use oxikube_testkit::StateCall;

use super::*;
use crate::hotbar::{HOTBAR_TABLE, HotbarStore};

/// Presses on `from`, drags past the drag threshold, moves over `to` and releases there.
fn drag(fx: &mut Fixture, from: &str, to: &str) {
    let from = center(
        fx.bounds(format!("hotbar-tile-{from}"))
            .expect("source tile"),
    );
    let to = center(fx.bounds(format!("hotbar-tile-{to}")).expect("target tile"));
    let vcx = &mut fx.vcx;
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_move(
        point(from.x + px(4.), from.y + px(10.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.update(|window, cx| window.draw(cx).clear(cx));
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
}

fn three(cx: &mut TestAppContext, state: Arc<FakeStatePort>) -> Fixture {
    Fixture::start(cx, &["a", "b", "c"], Dispatch::Record, state, |p| {
        p.connect_with_colour("a", None);
        p.connect_with_colour("b", None);
        p.connect_with_colour("c", None);
    })
}

fn saved(state: &Arc<FakeStatePort>) -> Vec<ClusterId> {
    let store = HotbarStore::new(state.clone(), "main").expect("store");
    block_on(store.load()).expect("load")
}

#[gpui::test]
fn dragging_a_tile_onto_another_moves_it_there(cx: &mut TestAppContext) {
    let mut fx = three(cx, Arc::new(FakeStatePort::new()));
    assert_eq!(fx.tiles(), ["a", "b", "c"]);
    drag(&mut fx, "c", "a");
    assert_eq!(fx.tiles(), ["c", "a", "b"]);
    drag(&mut fx, "c", "b");
    assert_eq!(
        fx.tiles(),
        ["a", "b", "c"],
        "dropping on the last tile moves it last"
    );
}

#[gpui::test]
fn a_drag_is_not_a_click(cx: &mut TestAppContext) {
    let mut fx = three(cx, Arc::new(FakeStatePort::new()));
    drag(&mut fx, "a", "c");
    assert!(
        fx.recorder.sent().is_empty(),
        "reordering sends no command: {:?}",
        fx.recorder.sent()
    );
}

#[gpui::test]
fn the_new_order_is_saved_once_per_drop(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    let mut fx = three(cx, state.clone());
    let puts = |state: &Arc<FakeStatePort>| {
        state
            .recorded_calls()
            .iter()
            .filter(|call| matches!(call, StateCall::TablePut(t, ..) if t.as_str() == HOTBAR_TABLE))
            .count()
    };
    assert_eq!(
        puts(&state),
        0,
        "nothing is written until the user places something"
    );
    drag(&mut fx, "c", "a");
    assert_eq!(puts(&state), 1);
    assert_eq!(saved(&state), [id("c"), id("a"), id("b")]);
}

#[gpui::test]
fn a_new_window_restores_the_order_the_user_gave(cx: &mut TestAppContext) {
    let state = Arc::new(FakeStatePort::new());
    {
        let mut first = three(cx, state.clone());
        drag(&mut first, "b", "a");
        assert_eq!(first.tiles(), ["b", "a", "c"]);
    }
    let mut second = three(cx, state);
    assert_eq!(second.tiles(), ["b", "a", "c"]);
}

#[gpui::test]
fn an_unreadable_saved_order_is_ignored(cx: &mut TestAppContext) {
    use oxikube_ports::{StateKey, StatePort as _, StateTable};
    let state = Arc::new(FakeStatePort::new());
    block_on(state.table_put(
        &StateTable::new(HOTBAR_TABLE).unwrap(),
        &StateKey::new("main").unwrap(),
        serde_json::json!({ "order": "garbage" }),
    ))
    .expect("put");
    let mut fx = three(cx, state);
    assert_eq!(
        fx.tiles(),
        ["a", "b", "c"],
        "the default order, and the hotbar works"
    );
}
