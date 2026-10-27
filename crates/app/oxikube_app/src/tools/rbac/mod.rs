//! The can-i tool and the `@rbac` mention for hosted agents and the can-i panel (E17-S08).
//!
//! "May this ServiceAccount delete pods in this namespace?" is `kubectl auth can-i`. One
//! [`CanIService`] answers it for the panel (`oxikube_rbac_ui`), the `k8s.can_i` tool and the
//! `rbac::CanI` command alike, so a user and an agent get the same answer with the same reason.
//!
//! | Piece | Where |
//! |---|---|
//! | who is asked as: yourself, a User, a Group, a ServiceAccount | [`CanISubject`] (`subject`) |
//! | one question in the words of `kubectl auth can-i`, and the check it becomes | [`CanIRequest`] (`request`) |
//! | the answer: decision, reason, and which review produced it | [`CanIAnswer`], [`CanIMethod`] (`answer`) |
//! | asking the cluster; prefilling a request from a selected object | [`CanIService`] (`service`) |
//! | `k8s.can_i`: the question as a read-only tool | [`CanITool`] (`tool`) |
//! | `@rbac/sa/<namespace>/<name>`: a ServiceAccount's bindings and verbs as a block | [`RbacContextProvider`] (`context`) |
//!
//! # Which review answers
//!
//! A check for yourself is a `SelfSubjectAccessReview`. A check as another subject is a
//! `SubjectAccessReview`, which needs permission to create one: without it the answer is a
//! `Forbidden` error that names the permission, never a guess. Every answer says which review it
//! came from ([`CanIMethod`]). The rules cache is not consulted: the API's own `reason` is the
//! source of truth for a self check. Reviews are `POST`s that store nothing, so none of this goes
//! through `MutationGuard`, and an impersonated subject lives in the request only: it is never
//! persisted.

mod answer;
mod context;
mod install;
mod request;
mod service;
mod subject;
mod summary;
mod tool;

#[cfg(test)]
mod tests;

pub use answer::{CanIAnswer, CanIMethod};
pub use context::{RBAC_PREFIX, RbacContextProvider};
pub use install::{CanIDeps, InstallError, install};
pub use request::CanIRequest;
pub use service::{CAN_I_DEADLINE, CanIService};
pub use subject::CanISubject;
pub use tool::{CAN_I, CanITool};
