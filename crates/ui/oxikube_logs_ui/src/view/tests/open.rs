//! `pod::ViewLogs` through the controller: what it asks (container, previous instance, follow,
//! tail length) is what the stream reads, also when the view is changed before its pod was read.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, SharedString, TestAppContext};
use oxikube_domain::log::LogRange;
use oxikube_testkit::Timeline;
use oxikube_workspace::{Item as _, ItemEvent};

use super::fixture::{Fx, lines};
use crate::view::{HEAD_LIMIT_BYTES, OpenLogs, TAIL_LINES};

fn title(fx: &mut Fx, view: &gpui::Entity<crate::LogView>) -> SharedString {
    fx.vcx
        .update(|_, cx: &mut App| view.read(cx).tab_content(cx).title)
}

#[gpui::test]
fn view_logs_of_the_previous_instance_reads_the_default_container(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let open = OpenLogs {
        previous: true,
        ..OpenLogs::default()
    };
    let view = fx.open_with(Timeline::immediate(lines(0, 3)), &open, |_, _| {});
    let opened = fx.opened();
    assert_eq!(opened.len(), 1, "one stream, opened once the pod was read");
    assert_eq!(opened[0].container.as_deref(), Some("app"), "the default");
    assert!(opened[0].previous && !opened[0].follow);
    assert_eq!(title(&mut fx, &view), "web-0/app (previous)");
}

#[gpui::test]
fn a_change_before_the_pod_is_read_opens_with_the_default_container(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open_with(
        Timeline::immediate(lines(0, 3)),
        &OpenLogs::default(),
        |view, cx| {
            view.set_range(LogRange::Head, cx);
            view.toggle_previous(cx);
        },
    );
    let opened = fx.opened();
    assert_eq!(
        opened.len(),
        1,
        "nothing opened before the pod named the container"
    );
    let last = &opened[0];
    assert_eq!(last.container.as_deref(), Some("app"));
    assert!(last.previous);
    assert_eq!(
        last.limit_bytes,
        Some(HEAD_LIMIT_BYTES),
        "the range asked meanwhile"
    );
    assert_eq!(fx.read(&view, |v| v.line_window().line_count()), 3);
}

#[gpui::test]
fn view_logs_reads_the_asked_tail_and_may_not_follow(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let open = OpenLogs {
        follow: false,
        tail_lines: Some(50),
        ..OpenLogs::default()
    };
    let view = fx.open_with(Timeline::immediate(lines(0, 3)), &open, |_, _| {});
    let first = fx.opened()[0].clone();
    assert_eq!(first.tail_lines, Some(50));
    assert!(!first.follow, "follow: false reads the tail once");

    // Asked again of the open view: it reads what is asked now.
    let again = OpenLogs {
        tail_lines: Some(20),
        ..OpenLogs::default()
    };
    let same = fx.open_with(Timeline::immediate(lines(0, 1)), &again, |_, _| {});
    assert_eq!(view, same);
    let last = fx.opened().last().cloned().unwrap();
    assert_eq!(last.tail_lines, Some(20));
    assert!(last.follow);

    // A range key reads the range's own tail again.
    fx.script(Timeline::immediate(lines(0, 1)));
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.set_range(LogRange::Last1m, cx);
            view.set_range(LogRange::Tail, cx);
        })
    });
    fx.settle();
    assert_eq!(fx.opened().last().unwrap().tail_lines, Some(TAIL_LINES));
}

#[gpui::test]
fn toggling_the_previous_instance_redraws_the_tab(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = fx.open(Timeline::immediate(lines(0, 3)).keep_open());
    fx.script(Timeline::immediate(lines(0, 1)));
    let events = Rc::new(RefCell::new(Vec::new()));
    let seen = events.clone();
    let _subscription = fx.vcx.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &ItemEvent, _| {
            seen.borrow_mut().push(*event);
        })
    });
    fx.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.toggle_previous(cx)));
    fx.settle();
    assert_eq!(*events.borrow(), [ItemEvent::UpdateTab]);
    assert_eq!(title(&mut fx, &view), "web-0/app (previous)");
}
