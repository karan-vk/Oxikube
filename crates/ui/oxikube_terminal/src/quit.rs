//! Work the terminal does when the app quits.

use gpui::App;

use crate::backend::local::cleanup_runtime_dir;

/// Runs [`cleanup_runtime_dir`] when the app quits. GPUI does not drop the entities (and so the
/// terminals) on quit, so without this the merged kubeconfig of a terminal that is still open
/// stays on disk, inline credentials included.
pub(crate) fn remove_runtime_dir_on_quit(cx: &mut App) {
    run_on_quit(cx, cleanup_runtime_dir);
}

/// Calls `work` once, synchronously, when the app quits.
fn run_on_quit(cx: &mut App, work: impl Fn() + 'static) {
    cx.on_app_quit(move |_| {
        work();
        std::future::ready(())
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::TestAppContext;

    use super::*;

    #[gpui::test]
    fn the_work_runs_when_the_app_quits_and_not_before(cx: &mut TestAppContext) {
        let runs = Rc::new(Cell::new(0));
        cx.update(|cx| {
            let counter = runs.clone();
            run_on_quit(cx, move || counter.set(counter.get() + 1));
            assert_eq!(runs.get(), 0);
            cx.shutdown();
        });
        assert_eq!(runs.get(), 1);
    }
}
