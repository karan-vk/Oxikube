//! Switching sources off and on: the view's hidden set, and that the streams keep running.

use std::time::Duration;

use futures::StreamExt as _;
use oxikube_testkit::Timeline;

use super::{Harness, line, texts, web, web_pod};

#[test]
fn toggling_a_source_notifies_and_does_not_stop_its_stream() {
    let mut h = Harness::new();
    h.seed([web(), web_pod("web-a"), web_pod("web-b")]);
    h.script(Timeline::immediate([line("web-a", 1, "a1")]).keep_open());
    h.script(Timeline::immediate([line("web-b", 2, "b2")]).keep_open());
    let session = h.open_web();
    let view = session.aggregate().clone();
    let mut changes = view.changes();

    let (hidden, version) = view.hidden();
    assert!(hidden.is_empty());

    view.toggle_source("web-a", None);
    let (hidden, after) = view.hidden();
    assert!(hidden.is_hidden("web-a", "app") && !hidden.is_hidden("web-b", "app"));
    assert_ne!(
        version, after,
        "a counter the viewer compares to know the set changed"
    );
    assert!(futures::executor::block_on(async { changes.next().await }).is_some());

    // Hidden sources keep streaming and keep their lines in the buffer: the view filters, the
    // buffer does not.
    h.run_for(Duration::from_secs(1));
    assert_eq!(h.logs.live_streams(), 2);
    assert_eq!(texts(&session), ["a1", "b2"]);

    view.toggle_source("web-a", None);
    assert!(view.hidden().0.is_empty());
}
