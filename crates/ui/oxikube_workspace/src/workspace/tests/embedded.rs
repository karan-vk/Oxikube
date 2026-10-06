//! A workspace embedded in another (a cluster's workspace inside its tab), and the strip left of
//! the docks.

use gpui::{AppContext as _, TestAppContext, div};

use super::*;
use crate::{DialogModal, Toast, ToastLevel};

fn embed(ws: &Entity<Workspace>, vcx: &mut VisualTestContext) -> Entity<Workspace> {
    vcx.update(|window, cx| {
        let layers = ws.read(cx).layers();
        let inner = cx.new(|cx| Workspace::embedded(layers, window, cx));
        // An embedded workspace is shown by an item of the outer one; here a bare item with it.
        inner
    })
}

#[gpui::test]
fn an_embedded_workspace_shares_the_outer_layers(cx: &mut TestAppContext) {
    let (outer, mut vcx) = workspace(cx);
    let inner = embed(&outer, &mut vcx);
    assert!(vcx.update(|_, cx| inner.read(cx).is_embedded()));
    assert!(!vcx.update(|_, cx| outer.read(cx).is_embedded()));
    vcx.update(|_, cx| {
        assert_eq!(
            inner.read(cx).modal_layer().entity_id(),
            outer.read(cx).modal_layer().entity_id()
        );
        assert_eq!(
            inner.read(cx).toast_layer().entity_id(),
            outer.read(cx).toast_layer().entity_id()
        );
        assert_eq!(
            inner.read(cx).status_bar().entity_id(),
            outer.read(cx).status_bar().entity_id()
        );
    });

    // A modal opened from the inner workspace is on the outer one's layer, where it is drawn.
    vcx.update(|window, cx| {
        let dialog = cx.new(|cx| DialogModal::new("From inside", cx));
        inner.update(cx, |ws, cx| ws.show_modal(dialog, window, cx));
        outer.update(cx, |ws, cx| {
            ws.show_toast(Toast::new(ToastLevel::Info, "hello"), cx);
        });
    });
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| {
        outer
            .read(cx)
            .modal_layer()
            .read(cx)
            .active_modal::<DialogModal>()
            .is_some()
    }));
}

#[gpui::test]
fn a_modal_opened_inside_an_embedded_workspace_is_drawn_over_the_whole_window(
    cx: &mut TestAppContext,
) {
    let (outer, mut vcx) = workspace(cx);
    let inner = embed(&outer, &mut vcx);
    let host = vcx.update(|_, cx| cx.new(|cx| HostItem::new(inner.clone(), cx)));
    vcx.update(|window, cx| outer.update(cx, |ws, cx| ws.open_item(host, window, cx)));
    vcx.run_until_parked();
    open(&inner, &mut vcx, "inside");
    assert!(
        bounds(&mut vcx, "item-inside").is_some(),
        "the inner workspace is drawn"
    );

    vcx.update(|window, cx| {
        let dialog = cx.new(|cx| DialogModal::new("From inside", cx));
        inner.update(cx, |ws, cx| ws.show_modal(dialog, window, cx));
    });
    vcx.run_until_parked();
    let layer = bounds(&mut vcx, "modal-layer").expect("the outer layer shows it");
    let dialog = bounds(&mut vcx, "dialog-modal").expect("the dialog is drawn");
    let window = vcx.update(|window, _| window.bounds());
    assert_eq!(
        layer.size, window.size,
        "it covers the window, not just the cluster's area"
    );
    assert!(layer.contains(&dialog.center()));
}

#[gpui::test]
fn the_strip_sits_left_of_the_docks_and_can_be_removed(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let before = bounds(&mut vcx, "item-a").expect("drawn");
    let strip = vcx.update(|_, cx| cx.new(|_| Strip));
    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.set_strip(Some(strip.into()), cx)));
    vcx.run_until_parked();
    let strip = bounds(&mut vcx, "test-strip").expect("the strip is drawn");
    let after = bounds(&mut vcx, "item-a").expect("drawn");
    assert_eq!(strip.origin.x, gpui::px(0.));
    assert!(
        after.origin.x >= strip.origin.x + strip.size.width,
        "the item starts after it"
    );
    assert!(
        after.size.width < before.size.width,
        "the strip takes its width from the docks"
    );

    vcx.update(|_, cx| ws.update(cx, |ws, cx| ws.set_strip(None, cx)));
    vcx.run_until_parked();
    assert!(bounds(&mut vcx, "test-strip").is_none());
    assert_eq!(
        bounds(&mut vcx, "item-a").expect("drawn").size.width,
        before.size.width
    );
}

struct Strip;

impl gpui::Render for Strip {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{InteractiveElement as _, Styled as _};
        div()
            .id("test-strip")
            .debug_selector(|| "test-strip".to_owned())
            .w(gpui::px(48.))
            .h_full()
    }
}

/// An item that shows a workspace.
struct HostItem {
    focus: gpui::FocusHandle,
    inner: Entity<Workspace>,
}

impl HostItem {
    fn new(inner: Entity<Workspace>, cx: &mut gpui::Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            inner,
        }
    }
}

impl gpui::EventEmitter<crate::ItemEvent> for HostItem {}

impl gpui::Focusable for HostItem {
    fn focus_handle(&self, _: &gpui::App) -> gpui::FocusHandle {
        self.focus.clone()
    }
}

impl gpui::Render for HostItem {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _};
        div().size_full().child(self.inner.clone())
    }
}

impl crate::Item for HostItem {
    fn tab_content(&self, _: &gpui::App) -> crate::TabContent {
        crate::TabContent::new("host")
    }
}
