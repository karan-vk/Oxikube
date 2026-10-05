//! Batching by size and by time, bounded buffering, and abort-on-drop.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::task::Poll;
use std::time::Duration;

use futures::TryStreamExt;
use futures::stream::{self, StreamExt};
use oxikube_domain::log::LogLine;
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::fake::{Chunk, FakeSource, follow, lines, texts, wire};
use crate::logs::stream::Sink;
use crate::logs::{LogsConfig, source::Reader};
use oxikube_ports::LogPort;

fn line(n: usize) -> LogLine {
    LogLine::new(super::fake::ts(0), "p", "app", format!("line {n}"))
}

#[tokio::test(start_paused = true)]
async fn a_full_batch_is_sent_and_the_rest_waits_for_the_timer() {
    let (tx, mut rx) = mpsc::channel(8);
    let config = LogsConfig {
        batch_size: 50,
        flush_interval: Duration::from_millis(100),
        ..LogsConfig::default()
    };
    let mut sink = Sink::new(tx, &config);
    for n in 0..120 {
        sink.push(line(n)).await.unwrap();
    }
    // Two full batches went out as lines arrived; 20 are pending with a deadline.
    assert_eq!(rx.recv().await.unwrap().unwrap().len(), 50);
    assert_eq!(rx.recv().await.unwrap().unwrap().len(), 50);
    assert!(rx.try_recv().is_err());
    assert!(sink.deadline().is_some());
    sink.flush().await.unwrap();
    assert_eq!(rx.recv().await.unwrap().unwrap().len(), 20);
    assert!(sink.deadline().is_none());
}

#[tokio::test(start_paused = true)]
async fn a_quiet_stream_is_flushed_after_the_interval_not_held_for_a_full_batch() {
    let source = FakeSource::new();
    // Three lines, then the server goes quiet but keeps the connection open.
    source.reply("p", "app", [lines(0..3), vec![Chunk::Hang]].concat());
    let mut stream = source
        .logs()
        .stream_logs("ns", "p", &follow())
        .await
        .unwrap();

    let started = Instant::now();
    let first = stream.next().await.unwrap().unwrap();

    assert_eq!(first.text, "line 0");
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(100) && waited < Duration::from_millis(200),
        "flushed by the timer after about 100 ms, got {waited:?}"
    );
    let rest = [
        stream.next().await.unwrap().unwrap(),
        stream.next().await.unwrap().unwrap(),
    ];
    assert_eq!(rest.map(|l| l.text), ["line 1", "line 2"]);
}

#[tokio::test(start_paused = true)]
async fn a_burst_is_delivered_in_order_through_many_batches() {
    let source = FakeSource::new();
    source.reply("p", "app", lines(0..1000));
    let opts = oxikube_ports::LogOptions::default().container("app");
    let items: Vec<_> = source
        .logs()
        .stream_logs("ns", "p", &opts)
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(texts(&items), super::fake::expect_lines(0..1000));
}

/// A reader that yields lines forever and counts how many the reader task pulled.
fn endless(pulled: Arc<AtomicUsize>) -> Reader {
    Box::pin(
        stream::unfold(0i64, move |n| {
            let pulled = Arc::clone(&pulled);
            async move {
                pulled.fetch_add(1, Ordering::SeqCst);
                Some((
                    Ok::<_, std::io::Error>(wire(n, &format!("line {n}"))),
                    n + 1,
                ))
            }
        })
        .into_async_read(),
    )
}

#[tokio::test(start_paused = true)]
async fn a_consumer_that_stops_reading_stops_the_reader() {
    use crate::logs::follow::{Follower, Target};
    use crate::logs::stream::ChannelStream;

    let pulled = Arc::new(AtomicUsize::new(0));
    let config = LogsConfig {
        batch_size: 10,
        channel_batches: 2,
        ..LogsConfig::default()
    };
    let (tx, rx) = mpsc::channel(config.channel_batches);
    let follower = Follower::new(
        Arc::new(FakeSource::new()),
        config.clone(),
        Target {
            namespace: "ns".into(),
            pod: "p".into(),
            container: "app".into(),
        },
        follow(),
        Sink::new(tx, &config),
        None,
    );
    let mut tasks = tokio::task::JoinSet::new();
    tasks.spawn(follower.run(Some(endless(Arc::clone(&pulled)))));
    let mut stream = ChannelStream::new(rx, tasks);

    // Let the reader run until it blocks on the full channel.
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    let before = pulled.load(Ordering::SeqCst);
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        pulled.load(Ordering::SeqCst),
        before,
        "the reader is blocked"
    );
    // Two queued batches, one in the blocked send, one filling: about 40 lines, not millions.
    assert!(before <= 50, "pulled {before} lines with nobody reading");

    // Reading frees it.
    for _ in 0..30 {
        stream.next().await.unwrap().unwrap();
    }
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    assert!(pulled.load(Ordering::SeqCst) > before);
}

#[tokio::test(start_paused = true)]
async fn dropping_the_stream_drops_the_connection() {
    let alive = Arc::new(());
    let weak: Weak<()> = Arc::downgrade(&alive);
    let source = FakeSource::new();
    // A response that holds `alive` for as long as the connection is open and never sends.
    let hold = Arc::clone(&alive);
    source.reply_raw(
        "p",
        "app",
        Box::pin(
            stream::poll_fn(move |_| {
                let _keep = &hold;
                Poll::<Option<std::io::Result<Vec<u8>>>>::Pending
            })
            .into_async_read(),
        ),
    );
    drop(alive);
    let stream = source
        .logs()
        .stream_logs("ns", "p", &follow())
        .await
        .unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        weak.upgrade().is_some(),
        "connection open while the stream lives"
    );

    drop(stream);
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        weak.upgrade().is_none(),
        "the reader task and its connection are gone"
    );
}
