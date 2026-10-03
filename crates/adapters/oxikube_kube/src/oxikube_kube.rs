//! `oxikube_kube` — layer: `adapters`.
//!
//! kube-rs adapter: tolerant kubeconfig loading, ClientPool per context with exec/OIDC auth, aggregated discovery, typed + dynamic APIs, reflector feeds, hand-rolled Table API feed, SSA/dry-run/patch/delete, subresources, kubectl-equivalent algorithms (trigger cronjob, rollout undo, drain), logs with reconnect, exec/attach/port-forward over ws, metrics (k8s-metrics), events.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
