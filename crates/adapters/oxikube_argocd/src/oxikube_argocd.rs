//! `oxikube_argocd` — layer: `adapters`.
//!
//! Argo CD integration adapters: CRD-direct backend (kopium types), argocd-server REST backend (SSE/NDJSON streams, cookie-auth terminal WS, token/CLI-config/SSO PKCE auth), core helper (argocd admin dashboard), Argo Rollouts CRD actions.
//!
//! See `README.md` in this crate and `docs/ARCHITECTURE.md` for the allowed
//! dependency direction. `cargo xtask lint-deps` enforces it.
