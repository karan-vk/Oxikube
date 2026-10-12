//! Secrets (E12-S11): reveal, decode, edit as plaintext, create.
//!
//! ```text
//! detail ─ x (secret::Reveal) ─> SecretService::read_decoded (a get on the Kubernetes runtime)
//!                                   └─> DecodedSecret, held in the detail view's memory only
//! editor ─ secret::EditDecoded ─> to_decoded(object) ─> buffer with plaintext under `data`
//! editor ─ resource::Apply ─────> SecretService::encode_manifest (base64 again) ─> ApplyService
//! dialog ─ secret::Create ──────> NewSecret::validate / manifest ─> an editor tab ─> resource::Apply
//! agent  ─ k8s.get_secret ──────> tools::k8s::GetSecretTool: key names and sizes; values only through its RevealPolicy
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `service` | [`SecretService`]: decode, summarise, read for reveal, build a new Secret, and encode the editor's manifest to apply |
//! | `summary` | [`SecretSummary`]: key names, sizes and the type's public facts (registries and usernames, the certificate) with no value |
//! | `tls` | [`TlsInfo`]: the subject, validity and names of a certificate (`x509-parser`), and the PEM checks of the create dialog |
//! | `create` | [`NewSecret`]: Opaque, Docker registry and TLS Secrets, validated field by field and built into a manifest |
//!
//! No decoded value is written to disk or logged: the types print `***`, the service logs the
//! object it read and the number of keys, and the audit records of `resource::Apply` name the
//! object and field manager, never the manifest. Every write is `resource::Apply`, through the
//! `MutationGuard` (risk low, blocked read-only, the dry-run diff with the values masked is the
//! confirmation).

mod create;
mod service;
mod summary;
mod tls;

pub use create::{Field, MAX_SECRET_BYTES, NewSecret, Problem};
pub use service::SecretService;
pub use summary::{KeyInfo, SecretSummary, TypeSummary};
pub use tls::{PemProblem, TlsInfo, check_private_key, parse_certificates};
