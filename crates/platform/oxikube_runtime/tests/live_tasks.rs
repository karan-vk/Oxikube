//! `live_tasks`: the count of `spawn_kube` futures that have not ended, per app.

use gpui::TestAppContext;
use oxikube_runtime::{init_deterministic, live_tasks, spawn_kube};

fn live(cx: &mut TestAppContext) -> usize {
    cx.update(|cx| live_tasks(cx))
}

#[gpui::test]
async fn the_count_follows_spawned_futures_until_they_finish_or_are_dropped(
    cx: &mut TestAppContext,
) {
    assert_eq!(live(cx), 0, "nothing before init");
    cx.update(init_deterministic);
    assert_eq!(live(cx), 0);

    // A future that never finishes on its own, like a pump over a quiet stream.
    let pending: Vec<_> = (0..5)
        .map(|_| cx.update(|cx| spawn_kube(cx, std::future::pending::<()>())))
        .collect();
    cx.run_until_parked();
    assert_eq!(live(cx), 5, "five futures wait");

    drop(pending);
    cx.run_until_parked();
    assert_eq!(live(cx), 0, "dropping the tasks ended them");

    // One that finishes counts down too, with nobody having to drop it first.
    let done = cx.update(|cx| spawn_kube(cx, async { 1 + 1 }));
    assert_eq!(done.await, Ok(2));
    assert_eq!(live(cx), 0, "a finished future is not alive");

    // A panic ends the future too.
    let panicked = cx.update(|cx| spawn_kube(cx, async { panic!("boom") }));
    let _ = panicked.await;
    assert_eq!(live(cx), 0, "a panicked future is not alive");
}

#[gpui::test]
fn a_tokio_task_is_alive_until_it_is_aborted(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(|cx| oxikube_runtime::init(cx).expect("runtime"));
    let task = cx.update(|cx| spawn_kube(cx, std::future::pending::<()>()));
    assert_eq!(live(cx), 1);
    drop(task);
    cx.run_until_parked();
    // The abort lands on a worker thread.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while live(cx) != 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the abort never landed"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
