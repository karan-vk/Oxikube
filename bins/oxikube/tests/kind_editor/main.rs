//! The manifest editor end to end on a real cluster (E10-S13), the epic's last proof: the app on
//! its real init order (kube adapter, SQLite state, Tokio runtime, headless main window), the kind
//! context connected, and the editor flow a user drives in the cluster tab, dry run, preview,
//! server-side apply and audit included, against the kind API server.
//!
//! | Module | Scenario |
//! |---|---|
//! | `template` | a ConfigMap from its template: refused in a read-only session (nothing sent, the guard's `Denied` audited), then applied, read back with `get`, every attempt audited without the manifest |
//! | `ssa` | "Edit" on a live Deployment, `spec.replicas` changed and applied with server-side apply: the `oxikube` field manager owns `spec.replicas`; then the conflict with `kubectl`'s server-side apply of the same field: the dry run lists `kubectl`, nothing is written, Force moves the field to `oxikube` |
//! | `secrets` | a Secret made in the "Create secret" dialog (Opaque rows, as text in its editor, base64 on the server), "Edit decoded" on it (the values as plaintext, one changed, applied, read back as base64, the binary value untouched), `secret::Reveal`'s read of the live values, and the same refused read-only; no value in the audit log (E12-S11) |
//! | `crd` | an instance of the sample CRD (`widgets.test.oxikube.dev`, installed by `cargo xtask kind-up`) checked against the cluster's own schema: a good one has no diagnostics, a bad one (an unknown field, a wrong type) has both; the editor refuses to send it, and the server's dry run of it names the same field, which lands on the same span |
//!
//! `cargo test -p oxikube --features integration --test kind_editor` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it each test returns at once. Every object lives in an
//! `oxi-test-<rand>` namespace the test creates and deletes (also when it fails); the pods of the
//! Deployments name a scheduler nobody runs, so they stay `Pending` and cost the node nothing.
//! Waits poll a condition with a deadline (`kind_common::wait`), never a fixed sleep.
#![cfg(feature = "integration")]

#[path = "../kind_common/mod.rs"]
mod kind_common;

mod crd;
mod objects;
mod secrets;
mod ssa;
mod template;
