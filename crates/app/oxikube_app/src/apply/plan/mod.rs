//! Many objects as one plan (E20-S03): `kubectl apply -f dir`, dry run first, through the guard.
//!
//! ```text
//! manifest docs ─ApplyService::plan─> ClusterPlan (sanitised, kind order, depends_on edges)
//! ClusterPlan ─dry_run_plan─> resource::ApplyPlan (dry run) ─> guard ─> handler: every object's dry run
//!                         <─ results stream: created / configured / unchanged / deferred / failed
//! the user reviews them
//! ClusterPlan ─apply_plan─> resource::ApplyPlan ─> guard (read-only, ONE confirmation at the riskiest
//!                       object's risk, audit) ─> handler:
//!                         the whole plan rehearsed again (stop on error: a failure stops here)
//!                         rank by rank, four objects at a time:
//!                           a new CRD Established (polled, 30 s) and discovery refreshed first
//!                           ObjectWrite::send (SSA dry run, SSA under `oxikube`), audited
//!                         <─ results stream as each object is done
//! ```
//!
//! | File | Holds |
//! |---|---|
//! | `types` | [`PlanTarget`], [`ClusterPlan`], [`ApplyOptions`], [`ObjectResult`], [`ObjectStatus`], [`PlanReport`] |
//! | `service` | `ApplyService::plan` / `dry_run_plan` / `apply_plan`: the command, the progress channel, the confirmation |
//! | `stream` | [`PlanStream`]: the dispatch driven as a stream of results |
//! | `build` | the plan from documents: sanitised, ordered by the domain's `ApplyPlan` over discovery |
//! | `handler` | the `resource::ApplyPlan` handler, [`MAX_PLAN_OBJECTS`] |
//! | `policy` | what the guard says about a plan: the confirmation text, the audit detail |
//! | `item` | one object of the plan: its write, its label, its edges |
//! | `run` | what one dispatch shares: the permit, the ports, the batch id; each object's audit record and report |
//! | `rehearse` | the dry-run pass: ready, deferred, no preview, failed, skipped |
//! | `execute` | the real pass: ranks in order, bounded parallelism, stop on error, cancel |
//! | `crd` | waiting for a new definition to be `Established`, then refreshing discovery |
//! | `detect` | created / configured / unchanged from the object before and after |
//!
//! # Safety
//!
//! There is no second write path. The plan is one mutating command, so the guard checks
//! read-only mode once for the whole plan (refused before any request, audited as denied), asks
//! one confirmation at [`Command::effective_risk`](oxikube_domain::command::Command::effective_risk)
//! (the riskiest object's [`classify`](oxikube_domain::safety::classify) risk: cluster-scoped,
//! RBAC and custom kinds, and force, raise it) and writes the command's own record with the
//! batch id. Each object is then written by [`ObjectWrite`](super::write) (the same code as
//! `resource::Apply`) through the guard's permit, whose writer re-checks read-only mode before
//! every request, and audited as `resource::Apply` with `batch=<id> via=resource::ApplyPlan`.
//! Objects may be Secrets: results, reports and audit records carry names and field paths only.

mod build;
mod crd;
mod detect;
mod execute;
mod handler;
mod item;
mod policy;
mod rehearse;
mod run;
mod service;
mod stream;
mod types;

pub use handler::MAX_PLAN_OBJECTS;
pub(crate) use handler::register;
pub(crate) use policy::{detail as audit_detail, summary};
pub use stream::PlanStream;
pub use types::{ApplyOptions, ClusterPlan, ObjectResult, ObjectStatus, PlanReport, PlanTarget};
