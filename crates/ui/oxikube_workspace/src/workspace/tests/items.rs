//! Opening, deduplicating, closing and reopening items.

use super::*;
use crate::actions::CloseActiveItem;

#[gpui::test]
fn open_item_creates_a_pane_and_displays_and_focuses_the_item(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    assert!(panes(&ws, &mut vcx).is_empty());
    assert!(vcx.update(|_, cx| ws.read(cx).is_blank()));

    let a = open(&ws, &mut vcx, "a");
    let panes = panes(&ws, &mut vcx);
    assert_eq!(panes.len(), 1);
    assert_eq!(panes[0].items(), [a]);
    assert_eq!(active_pane(&ws, &mut vcx).active_item(), Some(a));
    assert!(
        bounds(&mut vcx, "tab-a").is_some(),
        "the tab label is drawn"
    );
    assert!(bounds(&mut vcx, "item-a").is_some(), "the item is drawn");
    assert!(item_focused(&ws, &mut vcx, a));
}

#[gpui::test]
fn open_item_appends_to_the_active_pane_and_only_the_active_item_renders(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    let pane = active_pane(&ws, &mut vcx);
    assert_eq!(pane.items(), [a, b]);
    assert_eq!(pane.active_item(), Some(b));
    assert!(bounds(&mut vcx, "item-b").is_some());
    assert!(
        bounds(&mut vcx, "item-a").is_none(),
        "an inactive item keeps its state but is not laid out"
    );
    assert!(bounds(&mut vcx, "tab-a").is_some(), "its tab still is");
    vcx.update(|_, cx| {
        let ws = ws.read(cx);
        let a = ws
            .item(a)
            .and_then(|item| item.downcast::<TestItem>())
            .expect("a");
        let b = ws
            .item(b)
            .and_then(|item| item.downcast::<TestItem>())
            .expect("b");
        assert!(
            !a.read(cx).active && b.read(cx).active,
            "set_active follows the display"
        );
    });
}

#[gpui::test]
fn open_item_activates_an_open_item_instead_of_duplicating_it(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let first = open_with(&ws, &mut vcx, "pod", OpenOptions::default(), |item| {
        item.with_key("pod/default/web-0")
    });
    let other = open(&ws, &mut vcx, "other");
    assert_eq!(active_pane(&ws, &mut vcx).active_item(), Some(other));

    // A second item showing the same thing: the open one is activated, the new one dropped.
    let again = open_with(&ws, &mut vcx, "pod again", OpenOptions::default(), |item| {
        item.with_key("pod/default/web-0")
    });
    assert_eq!(again, first);
    let pane = active_pane(&ws, &mut vcx);
    assert_eq!(pane.items(), [first, other]);
    assert_eq!(pane.active_item(), Some(first));
    assert!(item_focused(&ws, &mut vcx, first));

    // The same entity opened twice is activated too.
    let handle = vcx.update(|_, cx| ws.read(cx).item(other).expect("open").boxed_clone());
    let reopened = vcx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_item_with(handle, OpenOptions::default(), window, cx)
        })
    });
    assert_eq!(reopened, other);
    assert_eq!(active_pane(&ws, &mut vcx).len(), 2);

    // Unless reuse is turned off.
    let options = OpenOptions {
        reuse_existing: false,
        ..OpenOptions::default()
    };
    let duplicate = open_with(&ws, &mut vcx, "pod", options, |item| {
        item.with_key("pod/default/web-0")
    });
    assert_ne!(duplicate, first);
    assert_eq!(active_pane(&ws, &mut vcx).len(), 3);
}

#[gpui::test]
fn open_item_at_an_index(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    open(&ws, &mut vcx, "b");
    let options = OpenOptions {
        index: Some(0),
        ..OpenOptions::default()
    };
    let c = open_with(&ws, &mut vcx, "c", options, |item| item);
    let pane = active_pane(&ws, &mut vcx);
    assert_eq!(titles(&ws, &mut vcx, &pane), ["c", "a", "b"]);
    assert_eq!(pane.active_item(), Some(c));
}

#[gpui::test]
fn close_item_runs_its_close_hook_and_honours_can_close(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    let pinned = open_with(
        &ws,
        &mut vcx,
        "pinned",
        OpenOptions::default(),
        |mut item| {
            item.closable = false;
            item
        },
    );
    let a_entity = vcx.update(|_, cx| ws.read(cx).item(a).and_then(|i| i.downcast::<TestItem>()));
    let closed_count = vcx.update(|_, cx| a_entity.expect("a").read(cx).closed.clone());

    let refused =
        vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(pinned, window, cx)));
    assert!(!refused, "an item that cannot close stays");
    assert_eq!(active_pane(&ws, &mut vcx).len(), 2);

    let closed = vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(a, window, cx)));
    vcx.run_until_parked();
    assert!(closed);
    assert_eq!(closed_count.get(), 1, "on_close ran once");
    assert_eq!(active_pane(&ws, &mut vcx).items(), [pinned]);
    assert!(vcx.update(|_, cx| ws.read(cx).item(a).is_none()));
    assert!(bounds(&mut vcx, "tab-a").is_none());
}

#[gpui::test]
fn closing_the_last_item_of_a_pane_removes_the_pane(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(a, window, cx)));
    vcx.run_until_parked();
    assert!(panes(&ws, &mut vcx).is_empty());
    assert!(vcx.update(|_, cx| ws.read(cx).active_pane(cx).is_none()));
    // The next item gets a new pane.
    let b = open(&ws, &mut vcx, "b");
    assert_eq!(panes(&ws, &mut vcx).len(), 1);
    assert_eq!(active_pane(&ws, &mut vcx).items(), [b]);
}

#[gpui::test]
fn reopen_closed_item_restores_it_where_it_was(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");
    open(&ws, &mut vcx, "c");
    let pane = active_pane(&ws, &mut vcx).id();

    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(b, window, cx)));
    vcx.run_until_parked();
    let closed = vcx.update(|_, cx| ws.read(cx).closed_items().peek().cloned());
    let closed = closed.expect("b is remembered");
    assert_eq!(closed.title.as_ref(), "b");
    assert_eq!(closed.pane, Some(pane));
    assert_eq!(closed.index, Some(1));

    let reopened =
        vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.reopen_closed_item(window, cx)));
    vcx.run_until_parked();
    let reopened = reopened.expect("b reopens");
    assert_ne!(reopened, b, "a new entity rebuilt from the descriptor");
    let active = active_pane(&ws, &mut vcx);
    assert_eq!(active.id(), pane);
    assert_eq!(titles(&ws, &mut vcx, &active), ["a", "b", "c"]);
    assert_eq!(active.active_item(), Some(reopened));
    assert!(vcx.update(|_, cx| ws.read(cx).closed_items().is_empty()));
    let nothing =
        vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.reopen_closed_item(window, cx)));
    assert!(nothing.is_none());
}

#[gpui::test]
fn close_active_item_action_and_close_button_feed_reopen(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    open(&ws, &mut vcx, "a");
    let b = open(&ws, &mut vcx, "b");

    // The action closes the displayed (focused) item.
    vcx.dispatch_action(CloseActiveItem);
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| ws.read(cx).item(b).is_none()));
    assert_eq!(vcx.update(|_, cx| ws.read(cx).closed_items().len()), 1);

    // The tab's own close button goes through the dock area; the workspace still records it.
    let c = open(&ws, &mut vcx, "c");
    assert_eq!(active_pane(&ws, &mut vcx).active_item(), Some(c));
    let close = bounds(&mut vcx, "dock-tab-close-button").expect("close buttons are shown");
    vcx.simulate_click(center(close), gpui::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        active_pane(&ws, &mut vcx).len(),
        1,
        "one tab was closed by its button"
    );
    assert_eq!(vcx.update(|_, cx| ws.read(cx).closed_items().len()), 2);
}

#[gpui::test]
fn update_tab_redraws_the_tab_label(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let a = open(&ws, &mut vcx, "a");
    assert!(bounds(&mut vcx, "tab-a").is_some());
    let item = vcx
        .update(|_, cx| ws.read(cx).item(a).and_then(|i| i.downcast::<TestItem>()))
        .expect("a");
    vcx.update(|_, cx| item.update(cx, |item, cx| item.set_title("renamed", cx)));
    vcx.run_until_parked();
    assert!(bounds(&mut vcx, "tab-renamed").is_some());
    assert!(bounds(&mut vcx, "tab-a").is_none());
}

#[gpui::test]
fn layout_changes_are_reported(cx: &mut TestAppContext) {
    let (ws, mut vcx) = workspace(cx);
    let events = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let _subscription = vcx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&ws, move |_, event: &WorkspaceEvent, _| {
            assert_eq!(*event, WorkspaceEvent::LayoutChanged);
            events.set(events.get() + 1);
        })
    });
    let a = open(&ws, &mut vcx, "a");
    assert!(events.get() > 0, "opening an item changes the layout");
    let before = events.get();
    vcx.update(|window, cx| ws.update(cx, |ws, cx| ws.close_item(a, window, cx)));
    vcx.run_until_parked();
    assert!(events.get() > before, "closing one too");
}
