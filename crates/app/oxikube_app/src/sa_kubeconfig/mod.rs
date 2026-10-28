//! ServiceAccount kubeconfig generation (E17-S09): the guarded `serviceaccount::GenerateKubeconfig`.
//!
//! ```text
//! ServiceAccount panel ─serviceaccount::GenerateKubeconfig─> CommandBus ─> guard ─┬─ read-only: denied (audited)
//! palette / row menu / agent: k8s.serviceaccount_generate_kubeconfig              ├─ confirm: simple (risk low)
//!                                                                                 └─> handler
//!   handler: get the account ─> cluster_endpoint (server + CA, ClusterSourcePort)
//!            ─> create_subresource(serviceaccounts/token) through the guard's writer
//!               └ cluster without TokenRequest: legacy token Secret, only when allowed
//!            ─> KubeconfigSpec::render ─> KubeconfigVault (memory only)
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `build` | [`KubeconfigSpec`]: the kubeconfig text, the token real or masked |
//! | `token` | the `TokenRequest` body and response, the legacy-Secret fallback |
//! | `vault` | [`KubeconfigVault`] and [`GeneratedKubeconfig`]: the result, in memory, for the panel |
//! | `handler` | [`register_commands`] |
//! | `spec` | what the guard says: confirmation text, audit detail |
//!
//! # The token is a secret
//!
//! It is requested through [`Mutation`](crate::guard::Mutation), carried as a
//! [`SecretText`](oxikube_domain::secret::SecretText) (`Debug` prints `***`, memory zeroed on
//! drop) and never put in the command's output, the audit record, a log field or a toast. The
//! file leaves memory only when the user copies it or saves it (the panel asks where, and writes
//! it through `FsPort::write_private`, mode 0600); this crate writes nothing.

mod build;
mod handler;
mod spec;
mod token;
mod vault;

#[cfg(test)]
mod tests;

pub use build::{KubeconfigSpec, MASK, Tokens};
pub use handler::{LONG_LIVED_WARNING, register_commands};
pub use spec::format_lifetime;
pub use token::{DEFAULT_EXPIRATION_SECONDS, MIN_EXPIRATION_SECONDS};
pub use vault::{GeneratedKubeconfig, KubeconfigVault, TokenSource};

pub(crate) use spec::{audit_detail, summary};
