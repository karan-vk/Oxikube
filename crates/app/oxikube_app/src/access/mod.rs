//! [`AccessService`]: what the current user may do, from cached rules reviews (E17-S02).
//!
//! A thin, cached layer over [`AccessReviewPort`](oxikube_ports::AccessReviewPort) for one
//! connected cluster. The sidebar, the action bars and the can-i tool (E17-S03, S08) ask it
//! instead of the port, so ten views asking about one namespace cost one request.
//!
//! * **Per-namespace rules cache with a TTL** ([`DEFAULT_TTL`], 60 s, measured on the injected
//!   `ClockPort`). [`AccessService::rules`] returns the `SelfSubjectRulesReview` of one namespace
//!   (`None` is the cluster-wide ask, marked [`namespaces_unseen`](oxikube_domain::access::AccessRules::namespaces_unseen)
//!   because the review sees one probe namespace's Roles only).
//! * **Single flight.** Concurrent asks for the same namespace share one port call.
//! * **Invalidation** on connect, namespace change and context switch:
//!   [`AccessService::invalidate`] and [`AccessService::invalidate_namespace`]; the
//!   [`AccessRegistry`] hands out one service per [`ClusterSession`](crate::session::ClusterSession)
//!   connection, starts a fresh one when the connection (so the port) is replaced, and
//!   ([`AccessRegistry::follow`]) drops it when the cluster is closed or disconnected.
//! * **Batch can-i.** [`AccessService::can_i_many`] answers from the cached rules whenever
//!   they can decide ([`AccessRules::decide`](oxikube_domain::access::AccessRules::decide)) and
//!   falls back to one `SelfSubjectAccessReview` only for the rest (`resourceNames` grants,
//!   partial reviews), with the number in flight capped at [`MAX_IN_FLIGHT`].
//! * **Render path.** [`AccessService::peek`] answers from the cache without waiting or
//!   allocating a request: zero network calls after the first review.
//! * **Fails open.** A failed review is an error or [`Unknown`](oxikube_domain::access::AccessDecision::Unknown),
//!   never a deny: only a clear review-based deny feeds greying out; the server stays the
//!   authority.
//! * **Accessible namespaces** ([`accessible_namespaces`]): the names to offer when the
//!   cluster-wide namespace list is forbidden. The cluster's `accessible_namespaces` setting
//!   (Lens `accessibleNamespaces`) wins when set; otherwise candidates are reviewed and the ones
//!   where the user may read something are kept.
//!
//! E17-S01 adds the [`SubjectIndex`]: the cluster's Roles, ClusterRoles and bindings kept from the
//! store's subscriptions, turned into normalised rules per subject (`oxikube_domain::rbac` reads and
//! normalises; `index`, `objects` and `grant` index them). It is plain data with no port.
//! E17-S04 adds the [`PolicyFeed`] that keeps one from the store's four RBAC subscriptions (abort-on-drop)
//! and the [`PolicyHub`] that shares it between the views that need it (a ServiceAccount's bound
//! roles, and the Users, Groups and matrix pages of [`rbac_subjects`](crate::rbac_subjects)): one
//! index per cluster. Its grants share the index's cached, normalised role rules (`Arc`), so a
//! query costs a lookup per binding, not a copy of every role.
//!
//! * **Soft gates** (E17-S03): [`AccessVisibility`] implements the sidebar's
//!   [`KindVisibility`](crate::kinds::KindVisibility) over the cached rules, and
//!   [`ActionAccess`](crate::actions::ActionAccess) greys a row action the user may not run.
//!   Both read [`AccessService::peek_stale`] (never waiting, keeping the last answer while a
//!   newer review is on its way) and treat anything but a definite deny as allowed.
//!
//! Reviews are `POST`s that create nothing, so none of this goes through `MutationGuard`.
//! Plain async Rust over a port: no gpui, no kube, nothing spawned.

mod cache;
mod feed;
mod grant;
mod hub;
mod index;
mod namespaces;
mod objects;
mod registry;
mod service;
mod visibility;

#[cfg(test)]
mod tests;

pub use feed::{PolicyFeed, PolicyStatus};
pub use grant::{Grant, ScopedRules};
pub use hub::PolicyHub;
pub use index::SubjectIndex;
pub use namespaces::{AccessibleNamespaces, NamespaceOrigin, accessible_namespaces};
pub use registry::AccessRegistry;
pub use service::{AccessService, DEFAULT_TTL, MAX_IN_FLIGHT};
pub use visibility::AccessVisibility;
