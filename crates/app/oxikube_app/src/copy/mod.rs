//! Copy a resource to another cluster or namespace (E20-S08).
//!
//! Moving a Deployment from staging to production, or a ConfigMap to another namespace, used to
//! be `kubectl get -o yaml`, hand-editing the server's fields out, and `kubectl apply`.
//! [`CopyService`] does it as one guarded flow: pick the target, review the target's server dry
//! run (what the sanitiser dropped, the diff, references the target lacks), resolve an object
//! that exists (overwrite, skip, rename), confirm, copy.
//!
//! ```text
//! picker ─> CopyService::preview ─resource::CopyTo (dry run)─> TARGET guard ─> handler
//!                                                    source: get (read, audited on the source)
//!                                                    target: kind / apiVersion / namespace served?
//!                                                            sanitize (E20-S02, Copy mode)
//!                                                            ObjectWrite (the apply pipeline)
//! review ──> CopyService::copy ─resource::CopyTo──────────> TARGET guard ─> same handler
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`CopyService`]: clusters to pick, connect on demand, namespaces, [`preview`](CopyService::preview), [`copy`](CopyService::copy) |
//! | `answer` | [`ReviewAnswer`], the review's confirmation, and the dry run's own |
//! | `classify` | dispatch and port failures turned into [`CopyError`] |
//! | `handler` | [`register_commands`]: the `resource::CopyTo` handler, the only code that writes a copy (through the apply pipeline's `ObjectWrite`) |
//! | `object` | the source object turned into the request body: sanitised, renamed, in the target namespace |
//! | `target` | what the target must have: the kind, the apiVersion, the namespace |
//! | `references` | the reference scan: ServiceAccounts, ConfigMaps, Secrets, claims the target lacks |
//! | `audit` | the guard's detail and confirmation text, and the record of the source read |
//! | `refusal` | why a copy was turned down, carried on the error; DNS-1123 name checks |
//! | `types` | [`CopyRequest`], [`CopyPreview`], [`CopyChoice`], [`CopyOutcome`], [`CopyError`] |
//!
//! # The guard is the target's
//!
//! `resource::CopyTo` names the **target** object, so the guard that runs it is the target
//! cluster's: its read-only flag refuses the copy before anything is read, its confirmation tier
//! applies (a Secret or an RBAC, custom or cluster-scoped kind is at least medium; a cluster-scoped
//! kind asks for the typed name, [`copy_risk`](oxikube_domain::command::copy_risk)), and its audit
//! log gets the record. The source cluster is only read, so its read-only flag is irrelevant; it
//! gets its own record of the read (the same command id on the source object, `copied_to=...`).
//! Both records say `copied_from` / `copied_to` and name objects only: never a field value.
//!
//! # Secrets
//!
//! Copying a Secret moves its data between clusters in memory only. It is masked (keyed
//! fingerprints) in everything the handler returns, and absent from audit details and logs.
//!
//! # A copy writes through the apply pipeline
//!
//! The write is `ObjectWrite` (server-side apply under the field manager `oxikube`, a dry run
//! first, the replace / create fallback), the same code `resource::Apply` runs; there is no second
//! writer. Immutable fields (a Deployment's `spec.selector`) are refused by the server on an
//! overwrite and shown as it says ([`CopyError::Invalid`]).

mod answer;
mod audit;
mod classify;
mod handler;
mod object;
mod references;
mod refusal;
mod service;
mod target;
mod types;

#[cfg(test)]
mod tests;

pub use answer::ReviewAnswer;
pub use handler::register_commands;
pub use service::{ClusterChoice, CopyService};
pub use types::{CopyChoice, CopyError, CopyOutcome, CopyPreview, CopyRequest, MissingReference};

pub(crate) use audit::{detail as audit_detail, summary};
