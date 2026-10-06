//! What the cluster sidebar needs from the cluster (E06-S10): which sections the user may list,
//! and which custom resources exist.
//!
//! | File | Holds |
//! |---|---|
//! | `review` | [`review_access`]: one rules review per selected namespace, merged (partial under All); [`AccessOutcome`] |
//! | `custom` | [`discover_custom_resources`]: CRD kinds from discovery, grouped by API group |
//!
//! # Visibility rule
//!
//! A section is shown when the user may `list` at least one of the kinds it covers
//! ([`AccessRules::any_offered`](oxikube_domain::access::AccessRules::any_offered)). Only a
//! definite denial hides: a partial review, a restricted (named objects only) grant and a review
//! that failed all keep the section.
//!
//! # Fail open
//!
//! When the review itself fails, [`AccessOutcome::Failed`] offers everything and carries the
//! reason for a visible warning: hiding a section by mistake is worse than showing a page the API
//! server then refuses (the server stays the authority on every request).
//!
//! # Reviews
//!
//! `review_access` asks, for a selection of named namespaces, for each of them (cluster-scoped
//! grants such as `nodes` and `namespaces` appear in every answer) and unions the answers: a
//! section is shown if any selected namespace allows it. Under `All` it asks cluster-wide, which
//! only sees one probe namespace's Roles, so the answer is marked partial and nothing is hidden
//! that it did not list. It is plain async over the [`AccessReviewPort`]; the caller runs it on
//! the Tokio bridge after the session is `Ready`, so it never delays connect.
//!
//! [`AccessReviewPort`]: oxikube_ports::AccessReviewPort

mod custom;
mod review;

#[cfg(test)]
mod tests;

pub use custom::{CustomKind, CustomResourceGroup, discover_custom_resources};
pub use review::{AccessOutcome, review_access};
