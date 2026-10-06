//! The harness (`oxikube_testkit::gpui_test`) over the real `AppState` (E05-S11): the pattern a
//! UI story copies when its views read `AppState` from the app.

use gpui::{App, Context, IntoElement, ParentElement as _, Render, TestAppContext, Window, div};
use oxikube_ports::{StateKey, StatePort as _};
use oxikube_testkit::{TestPorts, gpui_test::TestApp};
use serde_json::json;

use super::AppState;

/// A view that reads the active theme's name from the installed `AppState` while it renders.
struct ThemeLabel;

impl Render for ThemeLabel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = AppState::global(cx).active_theme(cx);
        div().child(theme.name.clone())
    }
}

fn state_key(name: &str) -> StateKey {
    StateKey::new(name).expect("a valid key")
}

#[gpui::test]
fn a_view_reads_the_installed_app_state(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    app.update(|cx| AppState::test(cx));
    let mut window = app.open_window(|_, _| ThemeLabel);
    // Rendering reads `AppState::global`, which panics when the state is not installed.
    window.draw_frame();
    assert!(window.read_root(|_, cx: &App| AppState::try_global(cx).is_some()));
}

#[gpui::test]
fn test_with_keeps_the_fakes_so_the_test_can_assert_on_them(cx: &mut TestAppContext) {
    let ports = TestPorts::seeded();
    let mut app = TestApp::new(cx);
    let state = app.update(|cx| AppState::test_with(cx, &ports));

    futures::executor::block_on(state.state().kv_set(&state_key("k"), json!(1))).unwrap();

    // The same fake the app holds: the write is visible through the handle the test kept.
    let calls = ports.state.recorded_calls();
    assert!(!calls.is_empty());
    assert_eq!(
        futures::executor::block_on(ports.state.kv_get(&state_key("k"))).unwrap(),
        Some(json!(1))
    );
}

#[gpui::test]
#[should_panic(expected = "before any other AppState::test call")]
fn test_with_after_another_state_panics(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    app.update(|cx| AppState::test(cx));
    app.update(|cx| AppState::test_with(cx, &TestPorts::empty()));
}
