//! [`ReleasesService`] (E16-S05): the Helm releases of a cluster for the Releases view, read
//! natively from the release Secrets and ConfigMaps (no `helm` binary), with live updates.
//!
//! ```text
//! ReleaseSource (the binary: sessions, stores, NativeHelmReader per connection)
//!        │ helm(cluster) ─▶ list / history / values / manifest / notes     (HelmPort, native)
//!        │ store(cluster) ─▶ watch(): Secrets + ConfigMaps `owner=helm` ─▶ one `()` per change
//!        └ kinds(cluster) ─▶ resources(): manifest ─▶ ResourceStore lease    (E16-S03 HelmService)
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `source` | [`ReleaseSource`]: what the service needs from the binary |
//! | `service` | [`ReleasesService`]: the reads, masked values and manifest, the reveal grants |
//! | `watch` | [`ReleaseWatch`]: the live-update stream (store feeds, namespace and connection changes) |
//! | `filter` | [`ReleaseFilter`]: the `/` filter grammar over the table's rows |
//! | `commands` | `helm::ShowValues`, `helm::ShowManifest`, `helm::RevealValues` |
//!
//! # Sensitive data
//!
//! Values, manifests and notes can hold credentials. Values are masked with
//! [`mask_values`](oxikube_domain::helm::mask_values) and a manifest's Secret documents with the
//! shared scrubber; both stay masked until the user's `helm::RevealValues` for that release,
//! which lasts until it is hidden again or the detail closes ([`ReleasesService::conceal`]).
//! Commands reply with masked text only, whoever asked, and a reveal reply carries a count, never
//! a value. Nothing is logged or written.
//!
//! # Cost
//!
//! Listing is the native reader's metadata list plus one header decode per changed release (its
//! cache keeps the rest), so 500 releases list in well under a second. Detail reads decode one
//! release on demand; masking and YAML rendering run on a blocking thread, never the caller's.

mod commands;
mod filter;
mod service;
mod source;
#[cfg(test)]
mod tests;
mod watch;

pub use commands::{COMMANDS, register_commands};
pub use filter::{FilterProblem, ReleaseFilter};
pub use service::{Disclosure, ManifestView, ReleasesService, ValuesView};
pub use source::ReleaseSource;
pub use watch::ReleaseWatch;
