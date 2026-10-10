//! What the palette reads from the focused view and the session when it opens.

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, TestAppContext,
    Window, div,
};
use oxikube_app::{CommandTarget, Selection};
use oxikube_domain::command::ViewContext;
use oxikube_domain::ids::Gvk;
use oxikube_workspace::{Item, ItemEvent, TabContent};

use super::{Fixture, cluster, declared_index, pod};
use crate::command_palette::{capture, selection_of};

#[test]
fn the_selection_is_none_one_many_or_mixed() {
    let target = CommandTarget::none();
    assert_eq!(selection_of(&target), Selection::none());
    let pods = target.clone().selecting(vec![pod("a")]);
    assert_eq!(
        selection_of(&pods),
        Selection::one(Gvk::new("", "v1", "Pod"))
    );
    let two = target.clone().selecting(vec![pod("a"), pod("b")]);
    assert_eq!(
        selection_of(&two),
        Selection::many(Gvk::new("", "v1", "Pod"), 2)
    );
    let mut node = pod("n");
    node.gvk = Gvk::new("", "v1", "Node");
    let mixed = target.selecting(vec![pod("a"), node]);
    assert_eq!(selection_of(&mixed), Selection::mixed(2));
}

#[gpui::test]
fn a_focused_table_gives_its_view_selection_and_session(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, declared_index(), &["web", "api"]);
    let env = f.env.clone();
    let captured = f.vcx.update(|window, cx| capture(window, &env, cx));
    assert_eq!(captured.context.view, ViewContext::Table);
    assert!(captured.context.cluster_active);
    assert!(!captured.context.read_only);
    assert_eq!(captured.context.selection.count(), 2);
    assert_eq!(captured.target.cluster, Some(cluster()));
    assert_eq!(captured.target.targets, [pod("web"), pod("api")]);

    f.env.set_read_only(true);
    let env = f.env.clone();
    let captured = f.vcx.update(|window, cx| capture(window, &env, cx));
    assert!(captured.context.read_only, "the session's read-only flag");
}

#[gpui::test]
fn without_a_session_nothing_is_active(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, declared_index(), &[]);
    *f.env.session.borrow_mut() = None;
    let env = f.env.clone();
    let captured = f.vcx.update(|window, cx| capture(window, &env, cx));
    assert!(!captured.context.cluster_active);
}

/// A view that is not a registered surface: only its key context says what it is.
struct Probe {
    focus: FocusHandle,
}

impl EventEmitter<ItemEvent> for Probe {}

impl Focusable for Probe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .id("probe")
            .key_context("LogView")
            .track_focus(&self.focus)
            .child("log")
    }
}

impl Item for Probe {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Log")
    }
}

#[gpui::test]
fn a_view_without_a_surface_is_known_by_its_key_context(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, declared_index(), &["web"]);
    let workspace = f.workspace.clone();
    let probe: Entity<Probe> = f.vcx.update(|window, cx| {
        let probe = cx.new(|cx| Probe {
            focus: cx.focus_handle(),
        });
        workspace.update(cx, |ws, cx| ws.open_item(probe.clone(), window, cx));
        let focus = probe.read(cx).focus.clone();
        focus.focus(window, cx);
        probe
    });
    f.settle();
    let env = f.env.clone();
    let captured = f.vcx.update(|window, cx| capture(window, &env, cx));
    assert_eq!(captured.context.view, ViewContext::Logs);
    assert!(
        captured.target.targets.is_empty(),
        "no objects from a log view"
    );
    assert_eq!(
        captured.target.cluster,
        Some(cluster()),
        "the active cluster"
    );
    drop(probe);

    // And the commands of the log view are offered there.
    f.open();
    assert!(
        f.listed()
            .contains(&oxikube_domain::command::CommandId::LOGS_FIND)
    );
}
