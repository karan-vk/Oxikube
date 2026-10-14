//! The keys and the commands that act on rows: moving the selection, opening the event's object,
//! copying a row (text and JSON), exporting the listed events.

use std::path::PathBuf;

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::event::Event;
use oxikube_domain::ids::{Gvk, ResourceRef};

use super::{Fx, cluster};
use crate::export::{row_json, row_text};

fn selected_uid(fx: &mut Fx) -> Option<String> {
    fx.model(|m| {
        m.selected_row()
            .and_then(|r| r.event.uid.as_deref().map(str::to_owned))
    })
}

#[gpui::test]
fn arrows_move_the_selection_and_enter_opens_the_events_object(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.open();
    fx.keys("down");
    assert_eq!(selected_uid(&mut fx).as_deref(), Some("w2"));
    fx.keys("j j");
    assert_eq!(selected_uid(&mut fx).as_deref(), Some("w1"));
    fx.keys("k");
    assert_eq!(selected_uid(&mut fx).as_deref(), Some("n2"));
    fx.keys("shift-g");
    assert_eq!(selected_uid(&mut fx).as_deref(), Some("n1"));
    fx.keys("g");
    assert_eq!(selected_uid(&mut fx).as_deref(), Some("w2"));
    fx.keys("enter");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::ResourceOpen {
            target: ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "cart-0"),
        })
    );
}

#[gpui::test]
fn copy_row_puts_the_row_on_the_clipboard_as_text_or_json(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.open();
    fx.keys("down");
    let event = fx.model(|m| m.selected_row().unwrap().event.clone());
    fx.keys("c");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::EventsCopyRow { cluster: cluster() })
    );
    let text = fx.vcx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(text.as_deref(), Some(row_text(&event).as_str()));
    assert!(
        text.unwrap()
            .starts_with("2026-01-01T00:00:04Z\tWarning\tFailedScheduling\tshop\tPod/cart-0")
    );
    fx.keys("shift-c");
    let json = fx.vcx.read_from_clipboard().and_then(|c| c.text()).unwrap();
    assert_eq!(json, row_json(&event));
    let back: Event = serde_json::from_str(&json).unwrap();
    assert_eq!(back.reason.as_ref(), "FailedScheduling");
}

#[gpui::test]
fn export_json_writes_the_listed_events_in_order_to_the_chosen_file(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.open();
    fx.keys("ctrl-z");
    fx.keys("ctrl-s");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::EventsExportJson { cluster: cluster() })
    );
    assert!(fx.vcx.did_prompt_for_new_path());
    let path = PathBuf::from("/tmp/out/events.json");
    let chosen = path.clone();
    fx.vcx.simulate_new_path_selection(move |_| Some(chosen));
    fx.settle();
    let written = fx.fs.file(&path).expect("written");
    let events: Vec<Event> = serde_json::from_slice(&written).unwrap();
    let uids: Vec<_> = events.iter().map(|e| e.uid.as_deref().unwrap()).collect();
    assert_eq!(uids, ["w2", "w1"], "the faults, newest first");
}

#[gpui::test]
fn cancelling_the_save_panel_writes_nothing(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.open();
    fx.click("events-export");
    assert!(fx.vcx.did_prompt_for_new_path());
    fx.vcx.simulate_new_path_selection(|_| None);
    fx.settle();
    assert!(fx.fs.file("/tmp/out/events.json").is_none());
}

#[gpui::test]
fn the_palette_sees_an_events_view_of_the_cluster(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.open();
    let surface = fx
        .vcx
        .update(|window, cx| oxikube_workspace::command_surface::focused(window, cx))
        .expect("the view is a command surface");
    assert_eq!(surface.view, oxikube_domain::command::ViewContext::Events);
    assert_eq!(surface.target.cluster, Some(cluster()));
}
