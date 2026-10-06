//! The tokio side of the bridge on a real runtime: synchronized-update deadline, stream end,
//! shutdown when the UI side goes away, and write merging.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt as _;
use futures::channel::mpsc as futures_mpsc;
use futures::stream;
use oxikube_ports::{BackendEvent, ExitStatus, TerminalBackend, TerminalSize};
use oxikube_testkit::fakes::FakeTerminalBackend;
use parking_lot::Mutex;
use tokio::sync::{mpsc, watch};

use super::pump::{GridUpdate, OutputGate, pump, write_loop};
use crate::grid::TermGrid;

struct Pumped {
    grid: Arc<Mutex<TermGrid>>,
    gate: OutputGate,
    updates: mpsc::Receiver<GridUpdate>,
    wake: Arc<AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

fn start(events: stream::BoxStream<'static, BackendEvent>) -> Pumped {
    let grid = Arc::new(Mutex::new(TermGrid::new(TerminalSize::new(20, 3), 100)));
    let (replies, _replies_rx) = futures_mpsc::unbounded::<Bytes>();
    let (tx, updates) = mpsc::channel(16);
    let wake = Arc::new(AtomicBool::new(false));
    let gate = OutputGate::default();
    let task = tokio::spawn(pump(
        events,
        grid.clone(),
        gate.clone(),
        replies,
        tx,
        wake.clone(),
    ));
    Pumped {
        grid,
        gate,
        updates,
        wake,
        task,
    }
}

fn row0(grid: &Mutex<TermGrid>) -> String {
    grid.lock().snapshot().row_text(0)
}

#[tokio::test]
async fn a_synchronized_update_that_never_ends_is_applied_at_its_deadline() {
    let events = stream::iter([BackendEvent::Output(Bytes::from_static(
        b"\x1b[?2026hheld back",
    ))])
    .chain(stream::pending())
    .boxed();
    let mut pumped = start(events);
    assert!(matches!(
        pumped.updates.recv().await,
        Some(GridUpdate::Changed)
    ));
    assert_eq!(row0(&pumped.grid), "", "held while the update is open");
    // What the UI does with `Changed`.
    pumped.wake.store(false, Ordering::Release);

    // vte's deadline is 150 ms; the pump applies the update then and wakes the UI again.
    let woke = tokio::time::timeout(Duration::from_secs(5), pumped.updates.recv()).await;
    assert!(matches!(woke, Ok(Some(GridUpdate::Changed))));
    assert_eq!(row0(&pumped.grid), "held back");
    pumped.task.abort();
}

#[tokio::test]
async fn output_waits_while_a_search_holds_the_gate() {
    let backend = FakeTerminalBackend::silent();
    let mut pumped = start(backend.output_stream());
    // A search is running: lines must not move under it.
    let searching = pumped.gate.lock().await;
    backend.output("during the search");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(row0(&pumped.grid), "", "output waits for the search");
    assert!(
        pumped.grid.try_lock().is_some(),
        "the waiting pump does not hold the grid lock"
    );

    drop(searching);
    let woke = tokio::time::timeout(Duration::from_secs(5), pumped.updates.recv()).await;
    assert!(matches!(woke, Ok(Some(GridUpdate::Changed))));
    assert_eq!(row0(&pumped.grid), "during the search");
    pumped.task.abort();
}

#[tokio::test]
async fn the_end_of_the_stream_is_an_exit() {
    let events = stream::iter([BackendEvent::Output(Bytes::from_static(b"x"))]).boxed();
    let mut pumped = start(events);
    assert!(matches!(
        pumped.updates.recv().await,
        Some(GridUpdate::Changed)
    ));
    assert!(matches!(
        pumped.updates.recv().await,
        Some(GridUpdate::Exited(status)) if status == ExitStatus::default()
    ));
    pumped.task.await.unwrap();
}

#[tokio::test]
async fn the_pump_stops_when_the_ui_side_is_gone() {
    let backend = FakeTerminalBackend::silent();
    let Pumped { updates, task, .. } = start(backend.output_stream());
    drop(updates);
    backend.output("after the view closed");
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("the pump returns")
        .unwrap();
}

#[tokio::test]
async fn queued_input_is_merged_into_one_write_and_resizes_go_out() {
    let backend = FakeTerminalBackend::silent();
    let (input, input_rx) = futures_mpsc::unbounded();
    let (resize, resize_rx) = watch::channel(TerminalSize::new(80, 24));
    for part in ["k", "get", " pods\r"] {
        input.unbounded_send(Bytes::from(part)).unwrap();
    }
    resize.send(TerminalSize::new(100, 30)).unwrap();
    resize.send(TerminalSize::new(90, 20)).unwrap();
    let shared: Arc<dyn TerminalBackend> = Arc::new(backend.clone());
    let writer = tokio::spawn(write_loop(shared, input_rx, resize_rx));

    // Closing the input ends the loop once everything queued is written.
    drop(input);
    tokio::time::timeout(Duration::from_secs(5), writer)
        .await
        .expect("the writer returns")
        .unwrap();
    assert_eq!(backend.resizes(), [TerminalSize::new(90, 20)]);
    assert_eq!(backend.writes(), [b"kget pods\r".to_vec()]);
    drop(resize);
}

#[gpui::test]
fn a_busy_grid_does_not_block_the_frame(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;
    cx.update(oxikube_runtime::init_deterministic);
    let backend = FakeTerminalBackend::silent();
    let boxed = Box::new(backend.clone());
    let terminal = cx.new(|cx| super::TerminalState::new(boxed, TerminalSize::new(20, 3), cx));
    backend.output("frame one");
    cx.run_until_parked();
    terminal.read_with(cx, |terminal, _| {
        let mut snapshot = crate::grid::TerminalSnapshot::default();
        assert!(terminal.try_snapshot_into(&mut snapshot));
        assert_eq!(snapshot.row_text(0), "frame one");

        // A search (or a parse slice) holds the grid: the frame keeps what it had.
        let held = terminal.grid.lock();
        assert!(!terminal.try_snapshot_into(&mut snapshot));
        assert_eq!(snapshot.row_text(0), "frame one");
        drop(held);
        assert!(terminal.try_snapshot_into(&mut snapshot));
    });
}
