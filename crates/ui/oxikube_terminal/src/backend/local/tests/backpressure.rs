//! The bounds that keep a flood from growing memory: a reader that stops at a full output queue
//! and a writer whose queue fills while the PTY does not take bytes.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use futures::channel::oneshot;
use tokio::sync::mpsc::{self, error::TrySendError};

use super::super::io::{ReaderState, WriteRequest, spawn_reader, spawn_writer};

/// A PTY that never ends and counts how often it was read.
struct Endless(Arc<AtomicUsize>);

impl Read for Endless {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.fetch_add(1, Ordering::SeqCst);
        buf.fill(b'x');
        Ok(buf.len())
    }
}

async fn settle() {
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn the_reader_stops_reading_when_the_output_queue_is_full() {
    const QUEUE: usize = 4;
    let reads = Arc::new(AtomicUsize::new(0));
    let (tx, mut rx) = mpsc::channel(QUEUE);
    let state = Arc::new(ReaderState::default());
    spawn_reader(Box::new(Endless(reads.clone())), tx.clone(), state.clone()).unwrap();

    // Nobody consumes. A reader without a bound would have read thousands of chunks by now.
    settle().await;
    let filled = reads.load(Ordering::SeqCst);
    // The queue, plus the chunk the reader holds while it waits for room.
    assert_eq!(tx.capacity(), 0, "the queue is full");
    assert!(
        (QUEUE..=QUEUE + 2).contains(&filled),
        "read {filled} chunks into a queue of {QUEUE}"
    );
    settle().await;
    assert_eq!(reads.load(Ordering::SeqCst), filled, "no reads while full");

    // Making room lets it continue, still bounded.
    rx.recv().await.unwrap();
    settle().await;
    let after = reads.load(Ordering::SeqCst);
    assert!(after > filled && after <= filled + 2, "{filled} -> {after}");

    // Dropping the backend releases a reader that waits on a full queue.
    state.stop.store(true, Ordering::Release);
    rx.recv().await.unwrap();
    settle().await;
    let stopped = reads.load(Ordering::SeqCst);
    settle().await;
    assert_eq!(reads.load(Ordering::SeqCst), stopped, "the reader exited");
}

/// A PTY that takes no bytes until it is released.
struct Gate {
    open: Arc<(Mutex<bool>, Condvar)>,
    written: Arc<AtomicUsize>,
}

impl Write for Gate {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let (lock, cvar) = &*self.open;
        let mut open = lock.lock().unwrap();
        while !*open {
            open = cvar.wait(open).unwrap();
        }
        self.written.fetch_add(buf.len(), Ordering::SeqCst);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn request(
    bytes: &[u8],
) -> (
    WriteRequest,
    oneshot::Receiver<oxikube_domain::OxiResult<()>>,
) {
    let (ack, done) = oneshot::channel();
    let bytes = bytes.to_vec();
    (WriteRequest { bytes, ack }, done)
}

#[tokio::test]
async fn the_write_queue_fills_while_the_pty_takes_no_bytes() {
    const QUEUE: usize = 3;
    let open = Arc::new((Mutex::new(false), Condvar::new()));
    let written = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel(QUEUE);
    spawn_writer(
        Box::new(Gate {
            open: open.clone(),
            written: written.clone(),
        }),
        rx,
    )
    .unwrap();

    // The first request is taken by the writer, which then blocks inside the PTY write.
    let (first, first_done) = request(b"a");
    tx.send(first).await.unwrap();
    settle().await;
    // Then exactly the queue's worth is accepted, and the next write has to wait.
    let mut acks = vec![first_done];
    for _ in 0..QUEUE {
        let (next, done) = request(b"b");
        tx.try_send(next).unwrap_or_else(|_| panic!("room"));
        acks.push(done);
    }
    let (overflow, _) = request(b"c");
    assert!(
        matches!(tx.try_send(overflow), Err(TrySendError::Full(_))),
        "a writer that is blocked must not accept more than the queue holds"
    );
    assert_eq!(written.load(Ordering::SeqCst), 0);

    // Releasing the PTY delivers everything that was accepted, in order.
    *open.0.lock().unwrap() = true;
    open.1.notify_all();
    for done in acks {
        done.await.unwrap().unwrap();
    }
    assert_eq!(written.load(Ordering::SeqCst), 1 + QUEUE);
}
