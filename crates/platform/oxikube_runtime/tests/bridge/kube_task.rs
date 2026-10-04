//! `spawn_kube`: results come back on the main thread, dropping the task aborts the work, panics
//! become errors, and init is idempotent.

use crate::DropFlag;
use gpui::TestAppContext;
use oxikube_domain::ErrorKind;
use oxikube_runtime::{
    KubeTaskError, RuntimeMode, handle, init, init_deterministic, mode, spawn_kube,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);

#[gpui::test]
async fn deterministic_result_arrives_on_main_thread_via_cx_spawn(cx: &mut TestAppContext) {
    cx.update(init_deterministic);
    let main = thread::current().id();

    let task = cx.spawn(async move |cx| {
        let result = spawn_kube(&cx, async { 40 + 2 }).await;
        (result, thread::current().id())
    });
    let (result, resumed_on) = task.await;

    assert_eq!(result, Ok(42));
    assert_eq!(
        resumed_on, main,
        "the awaiting foreground task resumes on the main thread"
    );
}

#[gpui::test]
async fn tokio_result_arrives_on_main_thread_via_cx_spawn(cx: &mut TestAppContext) {
    // Real tokio workers wake this task from their own threads.
    cx.executor().allow_parking();
    cx.update(|cx| init(cx).expect("tokio runtime starts"));
    assert_eq!(cx.update(|cx| mode(cx)), Some(RuntimeMode::Tokio));
    let main = thread::current().id();

    let task = cx.spawn(async move |cx| {
        let worker = spawn_kube(&cx, async {
            // Proves the future is on tokio: `Handle::current` panics outside a runtime.
            let _ = tokio::runtime::Handle::current();
            let me = thread::current();
            (me.id(), me.name().map(str::to_owned))
        })
        .await;
        (worker, thread::current().id())
    });
    let (worker, resumed_on) = task.await;
    let (worker_id, worker_name) = worker.expect("future completes");

    assert_ne!(worker_id, main, "the future ran off the main thread");
    assert!(
        worker_name
            .as_deref()
            .is_some_and(|n| n.starts_with("oxikube-tokio")),
        "ran on a bridge worker, got {worker_name:?}"
    );
    assert_eq!(
        resumed_on, main,
        "the result is delivered on the main thread"
    );
}

#[gpui::test]
fn dropping_task_aborts_the_tokio_future(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(|cx| init(cx).expect("tokio runtime starts"));
    let (started_tx, started_rx) = mpsc::channel();
    let (dropped_tx, dropped_rx) = mpsc::channel::<()>();

    let task = cx.update(|cx| {
        spawn_kube(cx, async move {
            // Dropping the future drops this sender, which disconnects `dropped_rx`.
            let _alive = dropped_tx;
            started_tx.send(()).ok();
            std::future::pending::<()>().await;
        })
    });
    started_rx
        .recv_timeout(WAIT)
        .expect("the tokio future started");
    assert_eq!(
        dropped_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty),
        "still running while the task is held"
    );

    drop(task);
    // GPUI drops the cancelled bridge future on its executor; that aborts the tokio task.
    cx.run_until_parked();

    assert_eq!(
        dropped_rx.recv_timeout(WAIT),
        Err(mpsc::RecvTimeoutError::Disconnected),
        "the tokio future was dropped (aborted)"
    );
}

#[gpui::test]
fn dropping_task_drops_the_future_in_deterministic_mode(cx: &mut TestAppContext) {
    cx.update(init_deterministic);
    let dropped = Arc::new(AtomicBool::new(false));
    let guard = DropFlag(dropped.clone());

    let task = cx.update(|cx| {
        spawn_kube(cx, async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        })
    });
    cx.run_until_parked();
    assert!(!dropped.load(Ordering::SeqCst), "held task keeps running");

    drop(task);
    cx.run_until_parked();
    assert!(
        dropped.load(Ordering::SeqCst),
        "dropped task drops its future"
    );
}

#[gpui::test]
async fn detached_task_runs_to_completion(cx: &mut TestAppContext) {
    cx.update(init_deterministic);
    let done = Arc::new(AtomicBool::new(false));
    let flag = done.clone();

    cx.update(|cx| spawn_kube(cx, async move { flag.store(true, Ordering::SeqCst) }))
        .detach();
    cx.run_until_parked();

    assert!(done.load(Ordering::SeqCst));
}

#[gpui::test]
async fn panic_becomes_a_redacted_error_in_deterministic_mode(cx: &mut TestAppContext) {
    cx.update(init_deterministic);
    let task = cx.update(|cx| {
        spawn_kube(cx, async {
            panic!("request failed: Authorization: Bearer s3cr3t-token-value");
        })
    });

    let Err(KubeTaskError::Panicked(message)) = task.await else {
        panic!("expected a panic error");
    };
    assert!(message.contains("request failed"), "{message}");
    assert!(
        !message.contains("s3cr3t-token-value"),
        "redacted: {message}"
    );
}

#[gpui::test]
async fn panic_becomes_an_error_in_tokio_mode(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(|cx| init(cx).expect("tokio runtime starts"));
    let task = cx.update(|cx| spawn_kube(cx, async { panic!("watch decoder bug") }));

    assert_eq!(
        task.await,
        Err(KubeTaskError::Panicked("watch decoder bug".into()))
    );
}

#[gpui::test]
fn init_is_idempotent_and_reports_its_mode(cx: &mut TestAppContext) {
    assert_eq!(cx.update(|cx| mode(cx)), None);
    cx.update(init_deterministic);
    // A later init must not replace the backend (that would drop a live runtime).
    cx.update(|cx| init(cx).expect("no-op"));
    assert_eq!(cx.update(|cx| mode(cx)), Some(RuntimeMode::Deterministic));
    assert!(
        cx.update(|cx| handle(cx)).is_none(),
        "no tokio handle in test mode"
    );
}

#[test]
fn task_errors_map_to_internal_oxi_errors() {
    let err: oxikube_domain::OxiError = KubeTaskError::Cancelled.into();
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert!(!err.is_retryable());
}
