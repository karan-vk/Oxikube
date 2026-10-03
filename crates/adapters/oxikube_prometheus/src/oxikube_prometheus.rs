//! `oxikube_prometheus` — layer: `adapters`.
//!
//! PromqlPort adapter: HTTP client, provider auto-detection (kube-prometheus, Lens stack, VictoriaMetrics, Mimir, OpenShift) and per-provider query catalogues.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
