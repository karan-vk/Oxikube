//! "Send to agent" (E08-S09): the selection (else the lines on screen) becomes a context block with
//! its source in the app's pending-context queue, which the agent panel drains once it exists.

use gpui::TestAppContext;
use jiff::Timestamp;
use oxikube_app::context::CollectingConsumer;
use oxikube_domain::command::Command;
use oxikube_domain::log::LogLine;
use oxikube_testkit::Timeline;

use super::fixture::{Fx, line, lines, pod_ref};

fn open(fx: &mut Fx, n: usize) -> gpui::Entity<crate::LogView> {
    fx.open(Timeline::immediate(lines(0, n)).keep_open())
}

#[gpui::test]
fn sending_a_selection_queues_a_block_with_its_source(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx, 10);
    fx.click_row(2, false);
    fx.click_row(4, true);
    fx.keys("a");

    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsSendToAgent { target: pod_ref() }),
        "the key is the command"
    );
    assert_eq!(fx.agent.pending(), 1, "it waits for the agent panel");
    let consumer = CollectingConsumer::new();
    let _attached = fx.agent.attach(consumer.clone());
    let [item] = &consumer.take()[..] else {
        panic!("one queued block")
    };
    let source = &item.source;
    assert_eq!(source.cluster, super::fixture::cluster());
    assert_eq!(source.cluster_name, "kind");
    assert_eq!(source.namespace, "shop");
    assert_eq!(source.subject, "web-0");
    assert_eq!(source.lines, 3);
    let (first, last) = source.span.expect("the time span of the lines");
    assert_eq!((first, last), (line(2).ts, line(4).ts));

    let body = &item.block.body;
    for needle in [
        "# namespace: shop",
        "# source: web-0",
        "# lines: 3",
        "INFO line 2\n",
        "INFO line 3\n",
        "ERROR line 4\n",
    ] {
        assert!(body.contains(needle), "{needle}\n{body}");
    }
    assert!(
        !body.contains("line 1\n") && !body.contains("line 5\n"),
        "{body}"
    );
    assert!(item.block.title.contains("shop/web-0") && item.block.title.contains("3 lines"));
    assert!(
        fx.toasts()
            .iter()
            .any(|t| t.starts_with("Queued 3 lines for the agent")),
        "{:?}",
        fx.toasts()
    );
}

#[gpui::test]
fn an_attached_agent_panel_receives_it_at_once_and_drains_what_waited(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx, 10);
    fx.click_row(0, false);
    fx.keys("a");
    assert_eq!(fx.agent.pending(), 1);

    let consumer = CollectingConsumer::new();
    let _attached = fx.agent.attach(consumer.clone());
    assert_eq!(consumer.len(), 1, "the queued block is drained on attach");
    assert_eq!(fx.agent.pending(), 0);

    fx.click_row(1, false);
    fx.keys("a");
    assert_eq!(consumer.len(), 2, "delivered at once while attached");
    assert!(
        fx.toasts().iter().any(|t| t == "Sent 1 line to the agent"),
        "{:?}",
        fx.toasts()
    );
}

#[gpui::test]
fn with_no_selection_it_sends_what_is_on_screen(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx, 5_000);
    fx.draw();
    let seqs = fx
        .read(&view, |v| v.viewport_seqs())
        .expect("lines on screen");
    fx.keys("a");
    let consumer = CollectingConsumer::new();
    let _attached = fx.agent.attach(consumer.clone());
    let [item] = &consumer.take()[..] else {
        panic!()
    };
    assert_eq!(item.source.lines as u64, seqs.end - seqs.start);
    assert!(item.block.body.contains(&line(4_999).text));
    assert!(
        item.source.lines < 200,
        "only the screen: {}",
        item.source.lines
    );
}

#[gpui::test]
fn secrets_are_masked_in_what_the_agent_receives_but_not_in_the_view(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let secret = "calling api Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.e30.abcdefghijklmnop done";
    let timeline = Timeline::immediate([
        LogLine::new(
            Timestamp::from_second(1_791_115_200).unwrap(),
            "web-0",
            "app",
            secret,
        ),
        LogLine::new(
            Timestamp::from_second(1_791_115_201).unwrap(),
            "web-0",
            "app",
            "ordinary",
        ),
    ])
    .keep_open();
    let view = fx.open(timeline);
    fx.click_row(0, false);
    fx.click_row(1, true);
    fx.keys("a");
    let consumer = CollectingConsumer::new();
    let _attached = fx.agent.attach(consumer.clone());
    let [item] = &consumer.take()[..] else {
        panic!()
    };
    assert!(
        !item.block.body.contains("eyJhbGciOiJIUzI1NiJ9"),
        "{}",
        item.block.body
    );
    assert!(item.block.body.contains("ordinary"));
    // The viewer is the user's own data: its buffer keeps the line as written.
    let kept = fx.read(&view, |v| v.line_window().retained_count());
    assert_eq!(kept, 2);
}

#[gpui::test]
fn an_oversized_selection_is_cut_with_a_note(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let timeline = Timeline::immediate((0..1_500).map(|i| {
        LogLine::new(
            Timestamp::from_second(1_791_115_200 + i).unwrap(),
            "web-0",
            "app",
            format!("{i:0>200}"),
        )
    }))
    .keep_open();
    let view = fx.open(timeline);
    fx.vcx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.click_line(0, false, cx);
            view.click_line(1_499, true, cx);
        })
    });
    fx.keys("a");
    let consumer = CollectingConsumer::new();
    let _attached = fx.agent.attach(consumer.clone());
    let [item] = &consumer.take()[..] else {
        panic!()
    };
    let body = &item.block.body;
    assert!(
        body.len() <= oxikube_domain::agent::MAX_CONTEXT_BLOCK_BYTES,
        "{}",
        body.len()
    );
    assert!(item.block.truncated);
    assert!(
        body.contains("further selected lines were left out"),
        "{}",
        &body[..400]
    );
    assert!(
        body.contains(&format!("{:0>200}", 0)),
        "the first lines are kept"
    );
    assert!(item.source.lines < 1_500);
}

#[gpui::test]
fn the_toolbar_button_sends_the_same_command(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    open(&mut fx, 6);
    fx.click_row(1, false);
    fx.click("log-send-to-agent");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsSendToAgent { target: pod_ref() })
    );
    assert_eq!(fx.agent.pending(), 1);
}

#[gpui::test]
fn an_empty_view_has_nothing_to_send(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    fx.open(Timeline::new().keep_open());
    fx.keys("a");
    assert_eq!(fx.agent.pending(), 0);
    assert!(
        fx.toasts()
            .iter()
            .any(|t| t == "There are no lines to send."),
        "{:?}",
        fx.toasts()
    );
}
