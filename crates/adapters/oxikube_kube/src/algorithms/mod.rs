//! kubectl-equivalent algorithms: the actions that have no single API call (E04-S07).
//!
//! `kubectl create job --from=cronjob/x`, `kubectl rollout history|undo` and `kubectl drain` are
//! client-side algorithms over several API calls. Oxikube needs the same behaviour without
//! shelling out to kubectl, so they live here as functions over a [`ResourcePort`](oxikube_ports::ResourcePort): no `kube`
//! types, no UI types, and unit-testable with an in-process API server.
//!
//! **The mutating functions are reachable only through `oxikube_app::mutation::MutationGuard`**
//! (ADR 0012); they execute and report, they decide nothing about read-only mode, confirmation
//! tiers or audit. The suggested tiers (set in `CommandMeta` by the stories that expose these)
//! are Low for [`trigger_cronjob`], Medium for [`rollout_undo`] and High, type-the-name, for
//! [`drain`].
//!
//! | Algorithm | Where | kubectl |
//! |---|---|---|
//! | [`trigger_cronjob`]: `jobTemplate` to a `Job` with `generateName` and an owner reference | `cronjob` (kdash port) | `create job --from=cronjob/x` |
//! | [`rollout_history`]: revisions with change cause and images | `rollout` | `rollout history` |
//! | [`rollout_undo`]: pick the ReplicaSet by revision, patch the template back | `rollout` | `rollout undo` |
//! | [`drain`]: plan, cordon, evict with PDB retry, wait for pods to go | `drain` | `drain` |
//!
//! # Ports, not adapters
//!
//! Every function takes `&dyn ResourcePort` (drain an `Arc` of it, because the progress stream
//! outlives the call), so the app and agents can run them over any `ResourcePort`, and the
//! tests run them on the testkit fakes as well as on [`KubeResources`](crate::KubeResources)
//! over a scripted API server. Errors are the port's: a missing object is `NotFound`, a refused
//! write `Forbidden` or `Conflict`, and the algorithms add `Validation` for input they refuse
//! (a CronJob without a job template, a paused Deployment, a drain a pod blocks).
//!
//! # Eviction and the client
//!
//! Drain treats an eviction a PodDisruptionBudget refuses as "wait and retry". That needs the
//! 429 at once: build the [`KubeResources`](crate::KubeResources) the drain runs on with
//! [`with_unretried_client`](crate::KubeResources::with_unretried_client), as for any eviction.

mod api;
mod cronjob;
mod drain;
mod rollout;
#[cfg(test)]
mod tests;

pub use cronjob::{INSTANTIATE_ANNOTATION, job_from_cronjob, trigger_cronjob};
pub use drain::{
    BlockReason, BlockedPod, DrainOptions, DrainPlan, DrainProgress, DrainSummary, PodRef,
    SkipReason, SkippedPod, drain, drain_to_completion, plan_drain,
};
pub use rollout::{
    REVISION_ANNOTATION, RolloutRevision, RolloutUndo, rollout_history, rollout_undo,
};
