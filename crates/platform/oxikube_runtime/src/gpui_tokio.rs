// Portions of this file are derived from Zed's `gpui_tokio` crate
// (https://github.com/zed-industries/zed), Copyright 2022 - 2025 Zed Industries, Inc.,
// licensed under the Apache License, Version 2.0 (http://www.apache.org/licenses/LICENSE-2.0).
// Modifications Copyright (c) Oxikube contributors: fallible `init`, idempotent init, a
// deterministic test backend, a named worker pool and a standalone runtime builder.
// SPDX-License-Identifier: Apache-2.0
// Source: crates/gpui_tokio/src/gpui_tokio.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! The tokio runtime GPUI hands Kubernetes work to (Zed's `gpui_tokio`, ported).
//!
//! GPUI's executors are not tokio, and kube-rs (hyper, rustls, tokio timers) needs a tokio reactor.
//! This module owns that runtime as a GPUI global so every context can reach it:
//!
//! - [`init`] builds a multi-thread runtime of [`DEFAULT_WORKER_THREADS`] workers that lives as
//!   long as the `App` (shut down in the background when the global drops);
//! - [`init_from_handle`] adopts a runtime the binary already owns (build it with
//!   [`build_runtime`] to share it with non-GPUI code such as xtask or tests);
//! - [`init_deterministic`] is the test mode: no tokio runtime and no OS threads at all. Work is
//!   polled on GPUI's background executor, which in `#[gpui::test]` is the deterministic test
//!   scheduler, so tests never trip "Detected activity on thread ...". tokio's `sync` primitives
//!   (channels, `Notify`, `Mutex`) work there; tokio I/O, timers and `tokio::spawn` do not, which
//!   is why GPUI tests drive app code through `oxikube_testkit` fakes rather than a cluster.
//!
//! Every `init*` call after the first is a no-op: replacing the global would drop the runtime and
//! abort every task in flight.

use gpui::{App, Global};
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use tokio::runtime::{Builder, Handle, Runtime};

/// Worker threads of the runtime [`init`] builds. Zed uses two so the process does not run two
/// full-size thread pools (GPUI's background executor already has one per core); kube-rs work is
/// I/O-bound and JSON decoding of large lists is batched by the feeds.
pub const DEFAULT_WORKER_THREADS: usize = 2;

/// Name prefix of the tokio worker threads (visible in Instruments, `perf` and panics).
pub const WORKER_THREAD_NAME: &str = "oxikube-tokio";

/// Which backend [`crate::spawn_kube`] runs futures on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    /// A tokio multi-thread runtime ([`init`] or [`init_from_handle`]).
    Tokio,
    /// GPUI's background executor, no tokio runtime ([`init_deterministic`]; tests only).
    Deterministic,
}

pub(crate) enum Backend {
    Tokio {
        /// `Some` when [`init`] built the runtime; `None` when the binary owns it.
        owned_runtime: Option<Runtime>,
        handle: Handle,
    },
    Deterministic,
}

/// The GPUI global holding the backend.
pub(crate) struct GlobalTokio {
    pub(crate) backend: Backend,
    /// How many `spawn_kube` futures of this app are alive (see [`crate::live_tasks`]).
    pub(crate) live: Arc<AtomicUsize>,
}

impl Global for GlobalTokio {}

impl Drop for GlobalTokio {
    fn drop(&mut self) {
        if let Backend::Tokio { owned_runtime, .. } = &mut self.backend
            && let Some(runtime) = owned_runtime.take()
        {
            // Never block the UI thread waiting for kube tasks to wind down.
            runtime.shutdown_background();
        }
    }
}

/// Builds the multi-thread runtime [`init`] uses, for a binary that wants to own it (and pass a
/// [`Handle`] to [`init_from_handle`] and to non-GPUI code).
pub fn build_runtime() -> io::Result<Runtime> {
    Builder::new_multi_thread()
        // Two executors live in this process; keep tokio's footprint small (see the constant).
        .worker_threads(DEFAULT_WORKER_THREADS)
        .thread_name(WORKER_THREAD_NAME)
        .enable_all()
        .build()
}

/// Starts the bridge on a runtime of its own ([`build_runtime`]), owned by the `App`.
///
/// Errors only when the OS refuses to create the worker threads. A second call is a no-op.
pub fn init(cx: &mut App) -> io::Result<()> {
    if cx.has_global::<GlobalTokio>() {
        return Ok(());
    }
    let runtime = build_runtime()?;
    let handle = runtime.handle().clone();
    cx.set_global(GlobalTokio {
        backend: Backend::Tokio {
            owned_runtime: Some(runtime),
            handle,
        },
        live: Arc::default(),
    });
    Ok(())
}

/// Starts the bridge on a runtime the caller owns. The caller keeps the runtime alive for as long
/// as the `App` runs. A second call (or a call after [`init`]) is a no-op.
pub fn init_from_handle(cx: &mut App, handle: Handle) {
    if cx.has_global::<GlobalTokio>() {
        return;
    }
    cx.set_global(GlobalTokio {
        backend: Backend::Tokio {
            owned_runtime: None,
            handle,
        },
        live: Arc::default(),
    });
}

/// Test mode: no tokio runtime, no OS threads; [`crate::spawn_kube`] polls futures on GPUI's
/// background executor (deterministic under `#[gpui::test]`). A second call is a no-op.
pub fn init_deterministic(cx: &mut App) {
    if cx.has_global::<GlobalTokio>() {
        return;
    }
    cx.set_global(GlobalTokio {
        backend: Backend::Deterministic,
        live: Arc::default(),
    });
}

/// The backend in use, or `None` before any `init*` call.
pub fn mode(cx: &App) -> Option<RuntimeMode> {
    cx.try_global::<GlobalTokio>().map(|g| match g.backend {
        Backend::Tokio { .. } => RuntimeMode::Tokio,
        Backend::Deterministic => RuntimeMode::Deterministic,
    })
}

/// The tokio runtime handle, for code that must enter the runtime itself (a `tokio::spawn` from a
/// non-GPUI callback, a blocking bridge in xtask). `None` before `init` and in deterministic mode.
/// Inside GPUI prefer [`crate::spawn_kube`], which ties the task's lifetime to a GPUI `Task`.
pub fn handle(cx: &App) -> Option<Handle> {
    match &cx.try_global::<GlobalTokio>()?.backend {
        Backend::Tokio { handle, .. } => Some(handle.clone()),
        Backend::Deterministic => None,
    }
}
