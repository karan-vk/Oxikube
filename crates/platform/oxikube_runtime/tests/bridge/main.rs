//! `#[gpui::test]` suite for the runtime bridge (E05-S01): one test binary, one module per area.
//!
//! Every test uses `init_deterministic` (no OS threads) except the ones that prove the real tokio
//! path; those call `cx.executor().allow_parking()` first because tokio's workers wake GPUI tasks
//! from foreign threads, which the deterministic scheduler otherwise rejects.

mod channel;
mod frame_paced;
mod kube_task;
mod notify;
mod render_gate;
mod task_slot;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Sets its flag when dropped: proves a future was dropped (cancelled) rather than completed.
pub struct DropFlag(pub Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
