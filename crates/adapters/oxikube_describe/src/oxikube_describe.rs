//! `oxikube_describe` — layer: `adapters`.
//!
//! The [`DescribePort`](oxikube_ports::DescribePort) adapters: `kubectl describe`-style text for
//! one object, rendered natively with [deskribe](https://crates.io/crates/deskribe) and, when the
//! kind is not covered or the user prefers it, by running the `kubectl` binary.
//!
//! | File | Holds |
//! |---|---|
//! | `preference` | [`Backend`], [`DescribeConfig`], [`DescribePreference`]: the user's choice, shared by every cluster's describer and changed at run time (settings hot reload) |
//! | `native` | [`NativeDescribe`]: deskribe over the connection's kube client; resolves the kind's plural through discovery |
//! | `kubectl` | [`KubectlDescribe`]: `kubectl describe` as a child process, never on the UI thread, killed when dropped |
//! | `describer` | [`Describer`]: the port the app gets, choosing between the two by the preference |
//! | `resolve` | the kind's plural through discovery, shared by both backends |
//! | `errors` | classification of deskribe's and kubectl's failures into the error taxonomy (redacted) |
//!
//! # Contract
//!
//! Read-only: neither backend writes to the cluster. A [`DescribeOutput`](oxikube_ports::DescribeOutput)
//! says which backend produced it. With [`Backend::Auto`] deskribe renders every kind it covers
//! (36 specialised kinds and the generic layout for custom resources) and `kubectl` is tried only
//! for a kind deskribe does not support; when `kubectl` is missing too the error is
//! [`Unsupported`](oxikube_domain::ErrorKind::Unsupported) and says how to fix it. No credential
//! reaches a command line: the child inherits the environment, `--context` and `--kubeconfig`
//! carry a name and a path, and every message is redacted before it is built.
//!
//! The native renderer is a dependency, not vendored code: its licence (Apache-2.0, with the
//! Kubernetes NOTICE) is listed in `THIRD_PARTY_NOTICES.md`.

mod describer;
mod errors;
mod kubectl;
mod native;
mod preference;
mod resolve;

pub use describer::Describer;
pub use kubectl::{KubectlDescribe, KubectlTarget};
pub use native::NativeDescribe;
pub use preference::{Backend, DescribeConfig, DescribePreference};

#[cfg(test)]
mod tests;
