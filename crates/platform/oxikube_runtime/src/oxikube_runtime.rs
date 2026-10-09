//! `oxikube_runtime` — layer: `platform`.
//!
//! The tokio <-> GPUI bridge every crate that talks to a cluster goes through (E05-S01,
//! non-negotiable 7), plus the `--perf` recorder (E01-S14).
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! # Modules
//!
//! - [`gpui_tokio`]: the tokio runtime as a GPUI global ([`init`], [`init_from_handle`],
//!   [`init_deterministic`] for tests, [`handle`]); ported from Zed's `gpui_tokio`.
//! - [`kube_task`]: [`spawn_kube`], which runs a future on that runtime and returns a GPUI task
//!   that aborts the tokio task when dropped.
//! - [`notify`]: [`notify_coalesced`], one `cx.notify()` per drawn frame however many
//!   times it is called.
//! - [`channel`]: [`batch_channel`], a bounded tokio channel whose receiver drains into an entity
//!   in batches.
//! - [`lazy`]: [`LazyService`], a service started on first use with `ensure_init` instead of at
//!   start-up (extension host, discovery, agent registry, update checker; E05-S13).
//! - [`fs`]: [`StdFs`], the `FsPort` over `std::fs` and `notify` (atomic writes, owner-only
//!   writes for files that may hold credentials; E06-S05).
//! - [`perf`]: the `--perf` recorder (frame times, feed throughput, `notify` counts), its JSONL
//!   flusher, the root-view frame hook and (feature `perf-harness`) the scripted headless frame
//!   driver behind `cargo xtask perf` (E01-S14, ADR 0013).
//!
//! # Rules (reviewers hold every crate to these)
//!
//! 1. **Nothing blocks the UI thread.** No I/O, process spawn, `std::thread::sleep`, contended
//!    lock or `block_on` in a view, an action handler or a foreground task. Kubernetes and other
//!    network work runs on tokio through [`spawn_kube`]; CPU-heavy work on
//!    `cx.background_spawn`.
//! 2. **Work is owned.** A [`KubeTask`] (like every GPUI `Task`) cancels its work when dropped:
//!    store it in the entity that wants the result so closing the view cancels the request. Only
//!    `.detach()` work that must finish with nobody listening.
//! 3. **Never let a task drop itself.** A task stored in `self.task` and cleared from inside
//!    that same task (`this.update(cx, |this, _| this.task = None)`) is dropped while it runs,
//!    which cancels it at its next `.await`: everything after that line silently never happens.
//!    When the task must mark itself finished, use a flag and `.detach()` instead:
//!
//!    ```ignore
//!    // Naive (broken): the task cancels itself at the first await after clearing its slot.
//!    self.refresh = Some(cx.spawn(async move |this, cx| {
//!        this.update(cx, |this, _| this.refresh = None).ok();
//!        let pods = spawn_kube(cx, list_pods()).await; // never resumes
//!    }));
//!
//!    // Flag + detach: the flag guards re-entry, the task clears it when done.
//!    if self.refreshing { return; }
//!    self.refreshing = true;
//!    cx.spawn(async move |this, cx| {
//!        let pods = spawn_kube(cx, list_pods()).await;
//!        this.update(cx, |this, cx| { this.refreshing = false; this.apply(pods, cx); }).ok();
//!    })
//!    .detach();
//!    ```
//!
//!    The detached task still stops touching the view once it is released (`this.update` fails),
//!    but its Kubernetes work runs to completion; prefer an owned task whenever the slot does not
//!    need clearing from inside. `tests/bridge/task_slot.rs` demonstrates both behaviours.
//! 4. **Streams coalesce.** Feeds batch through [`batch_channel`] and redraw with
//!    [`notify_coalesced`], never one `cx.notify()` per event.
//! 5. **GPUI tests are deterministic.** Call [`init_deterministic`], never [`init`], in
//!    `#[gpui::test]`: a real runtime wakes GPUI tasks from tokio's threads, which the test
//!    scheduler rejects ("Detected activity on thread ..."). A test that genuinely needs tokio
//!    I/O calls `cx.executor().allow_parking()` and says why.
//!
//! # Init order
//!
//! `bins/oxikube` calls [`init`] (or [`init_from_handle`] with a runtime it built with
//! [`build_runtime`]) before any crate that spawns Kubernetes work (E05-S09 documents the order).

pub mod channel;
pub mod fs;
pub mod gpui_tokio;
pub mod kube_task;
pub mod lazy;
pub mod notify;
pub mod perf;

pub use channel::{
    BatchReceiver, BatchSender, DEFAULT_BATCH_LIMIT, DEFAULT_CHANNEL_CAPACITY, batch_channel,
};
pub use fs::StdFs;
pub use gpui_tokio::{
    RuntimeMode, build_runtime, handle, init, init_deterministic, init_from_handle, mode,
};
pub use kube_task::{KubeTask, KubeTaskError, live_tasks, spawn_kube};
pub use lazy::{LazyService, LazyServices, StartedService};
pub use notify::{
    FRAME_INTERVAL, FRAME_STALL, NotifyCoalescedExt, notify_coalesced, notify_pending,
};
