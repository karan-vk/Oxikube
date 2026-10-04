//! `batch_channel`: queued items reach the entity in batches, in order, with backpressure, and the
//! drain task ends with its senders or its entity.

use gpui::{AppContext as _, Entity, Task, TestAppContext};
use oxikube_runtime::{BatchReceiver, batch_channel, init_deterministic, spawn_kube};

/// A view collecting what the drain delivers.
struct Sink {
    batches: Vec<Vec<u32>>,
    drain: Task<()>,
}

impl Sink {
    fn items(&self) -> Vec<u32> {
        self.batches.iter().flatten().copied().collect()
    }
}

fn sink(cx: &mut TestAppContext, rx: BatchReceiver<u32>) -> Entity<Sink> {
    cx.new(|cx| Sink {
        batches: Vec::new(),
        drain: rx.drain_into(cx, |sink: &mut Sink, batch, _| {
            sink.batches.push(batch.collect())
        }),
    })
}

#[gpui::test]
fn queued_items_arrive_in_one_batch(cx: &mut TestAppContext) {
    let (tx, rx) = batch_channel(16);
    let sink = sink(cx, rx);
    for i in 1..=5 {
        tx.try_send(i).expect("capacity");
    }
    cx.run_until_parked();

    sink.read_with(cx, |sink, _| {
        assert_eq!(sink.batches, vec![vec![1, 2, 3, 4, 5]])
    });
}

#[gpui::test]
fn batch_limit_splits_batches(cx: &mut TestAppContext) {
    let (tx, rx) = batch_channel(16);
    let sink = sink(cx, rx.with_batch_limit(2));
    for i in 1..=5 {
        tx.try_send(i).expect("capacity");
    }
    cx.run_until_parked();

    sink.read_with(cx, |sink, _| {
        assert_eq!(sink.batches, vec![vec![1, 2], vec![3, 4], vec![5]]);
    });
}

#[gpui::test]
fn bridge_producer_with_backpressure_delivers_everything_in_order(cx: &mut TestAppContext) {
    cx.update(init_deterministic);
    let (tx, rx) = batch_channel(64);
    let sink = sink(cx, rx);

    let producer = cx.update(|cx| {
        spawn_kube(cx, async move {
            for i in 0..10_000 {
                tx.send(i).await.map_err(|_| "receiver gone")?;
            }
            Ok::<_, &str>(())
        })
    });
    cx.run_until_parked();

    assert!(
        producer.is_ready(),
        "producer finished despite the 64-item bound"
    );
    sink.read_with(cx, |sink, _| {
        assert_eq!(sink.items(), (0..10_000).collect::<Vec<_>>());
        assert!(sink.batches.len() > 1, "delivered in batches");
        assert!(sink.batches.iter().all(|b| b.len() <= 64));
    });
}

#[gpui::test]
fn drain_ends_when_every_sender_is_dropped(cx: &mut TestAppContext) {
    let (tx, rx) = batch_channel(4);
    let sink = sink(cx, rx);
    tx.try_send(1).expect("capacity");
    cx.run_until_parked();
    sink.read_with(cx, |sink, _| assert!(!sink.drain.is_ready()));

    drop(tx);
    cx.run_until_parked();
    sink.read_with(cx, |sink, _| {
        assert!(sink.drain.is_ready(), "closed channel ends the drain task");
        assert_eq!(sink.items(), vec![1]);
    });
}

#[gpui::test]
fn dropping_the_entity_closes_the_channel(cx: &mut TestAppContext) {
    let (tx, rx) = batch_channel::<u32>(4);
    let sink = sink(cx, rx);
    cx.run_until_parked();

    drop(sink);
    // GPUI releases dropped entities when it next flushes effects; that drops the drain task.
    cx.update(|_| {});
    cx.run_until_parked();
    assert!(tx.is_closed(), "the drain task and its receiver are gone");
}
