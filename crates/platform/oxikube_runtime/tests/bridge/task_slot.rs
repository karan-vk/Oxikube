//! The self-dropping task pitfall (crate docs, rule 3): a task that clears its own slot cancels
//! itself; the flag + detach pattern does not.

use gpui::{AppContext as _, Context, Entity, Task, TestAppContext};
use oxikube_runtime::{init_deterministic, spawn_kube};

/// A view that refreshes something through the bridge.
#[derive(Default)]
struct Refresher {
    /// Naive pattern: the in-flight task, cleared by the task itself.
    slot: Option<Task<()>>,
    /// Flag + detach pattern: guards re-entry; cleared by the task when it finishes.
    refreshing: bool,
    /// What the refresh produced; `None` until the task reaches its end.
    result: Option<u32>,
    /// How many flag + detach refreshes ran to completion.
    completed_runs: u32,
}

impl Refresher {
    /// Broken on purpose: clears `slot` from inside the task it holds.
    fn refresh_naive(&mut self, cx: &mut Context<Self>) {
        self.slot = Some(cx.spawn(async move |this, cx| {
            // Dropping the stored task from inside it cancels it at the next await below.
            this.update(cx, |this, _| this.slot = None).ok();
            let value = spawn_kube(cx, async { 7 }).await.unwrap_or_default();
            this.update(cx, |this, _| this.result = Some(value)).ok();
        }));
    }

    /// The documented pattern: a flag instead of a slot, and a detached task.
    fn refresh_flag_and_detach(&mut self, cx: &mut Context<Self>) {
        if self.refreshing {
            return;
        }
        self.refreshing = true;
        cx.spawn(async move |this, cx| {
            let value = spawn_kube(cx, async { 7 }).await.unwrap_or_default();
            this.update(cx, |this, _| {
                this.refreshing = false;
                this.result = Some(value);
                this.completed_runs += 1;
            })
            .ok();
        })
        .detach();
    }
}

fn refresher(cx: &mut TestAppContext) -> Entity<Refresher> {
    cx.update(init_deterministic);
    cx.new(|_| Refresher::default())
}

#[gpui::test]
fn task_that_clears_its_own_slot_cancels_itself(cx: &mut TestAppContext) {
    let view = refresher(cx);
    view.update(cx, |view, cx| view.refresh_naive(cx));
    cx.run_until_parked();

    view.read_with(cx, |view, _| {
        assert!(view.slot.is_none(), "the task cleared its slot");
        assert_eq!(view.result, None, "and was cancelled before finishing");
    });
}

#[gpui::test]
fn flag_and_detach_task_runs_to_completion(cx: &mut TestAppContext) {
    let view = refresher(cx);
    view.update(cx, |view, cx| view.refresh_flag_and_detach(cx));
    view.read_with(cx, |view, _| assert!(view.refreshing));
    cx.run_until_parked();

    view.read_with(cx, |view, _| {
        assert!(!view.refreshing, "the task cleared its own flag");
        assert_eq!(view.result, Some(7), "and finished its work");
    });
}

#[gpui::test]
fn naive_and_flag_patterns_contrast(cx: &mut TestAppContext) {
    let naive = refresher(cx);
    let flagged = cx.new(|_| Refresher::default());
    naive.update(cx, |view, cx| view.refresh_naive(cx));
    flagged.update(cx, |view, cx| view.refresh_flag_and_detach(cx));
    cx.run_until_parked();

    let naive_result = naive.read_with(cx, |view, _| view.result);
    let flagged_result = flagged.read_with(cx, |view, _| view.result);
    assert_eq!((naive_result, flagged_result), (None, Some(7)));
}

#[gpui::test]
fn flag_guards_reentry(cx: &mut TestAppContext) {
    let view = refresher(cx);
    view.update(cx, |view, cx| {
        view.refresh_flag_and_detach(cx);
        view.refresh_flag_and_detach(cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert_eq!(view.completed_runs, 1, "second call was a no-op")
    });
    // A second refresh after completion is allowed again.
    view.update(cx, |view, cx| view.refresh_flag_and_detach(cx));
    view.read_with(cx, |view, _| assert!(view.refreshing));
    cx.run_until_parked();
    view.read_with(cx, |view, _| {
        assert!(!view.refreshing);
        assert_eq!(view.completed_runs, 2);
    });
}
