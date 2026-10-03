//! `oxikube_runtime` — layer: `platform`.
//!
//! tokio <-> GPUI bridge (gpui_tokio), spawn_kube with abort-on-drop, frame-coalesced notify helpers, channels.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
//!
//! # Modules
//!
//! - [`perf`]: the `--perf` recorder (frame times, feed throughput, `notify` counts), its JSONL
//!   flusher, the root-view frame hook and (feature `perf-harness`) the scripted headless frame
//!   driver behind `cargo xtask perf` (E01-S14, ADR 0013).

pub mod perf;
