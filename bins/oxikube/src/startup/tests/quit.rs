//! The log is stopped after the other quit observers' futures have logged.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui::TestAppContext;

use crate::startup::quit::flush_on_quit;

type Events = Rc<RefCell<Vec<&'static str>>>;

#[gpui::test]
fn the_log_is_flushed_after_a_later_observer_logs_from_its_future(cx: &mut TestAppContext) {
    let events = Events::default();
    cx.update(|cx| {
        // The app registers the flush last; the other work (the layout save) logs from the future
        // it returns, after awaiting its store.
        let sink = events.clone();
        cx.on_app_quit(move |cx| {
            sink.borrow_mut().push("sync part");
            let store = cx.background_executor().timer(Duration::from_millis(10));
            let sink = sink.clone();
            async move {
                store.await;
                sink.borrow_mut().push("logs from the future");
            }
        })
        .detach();
        let sink = events.clone();
        flush_on_quit(cx, Duration::from_millis(50), move || {
            sink.borrow_mut().push("flush");
        });
        cx.shutdown();
    });
    assert_eq!(
        *events.borrow(),
        ["sync part", "logs from the future", "flush"]
    );
}

#[gpui::test]
fn the_flush_runs_once(cx: &mut TestAppContext) {
    let events = Events::default();
    cx.update(|cx| {
        let sink = events.clone();
        flush_on_quit(cx, Duration::ZERO, move || sink.borrow_mut().push("flush"));
        cx.shutdown();
        cx.shutdown();
    });
    assert_eq!(*events.borrow(), ["flush"]);
}
