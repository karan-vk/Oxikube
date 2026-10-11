//! Jobs (E12-S08): the kind's spec and its row actions, suspend and resume.
//!
//! The columns (Status, Completions, Duration, Conditions, Age) are the shared core catalogue's.
//! Suspending a Job makes its controller delete the running pods until it is resumed; k9s offers
//! that as a plugin, here it is built in. A Job that finished cannot be resumed (the handler says
//! so). `s` suspends or resumes the cursor row's Job.

use gpui::App;
use oxikube_app::actions::KindFilter;
use oxikube_app::{KindMeta, RowActionSpec, Section};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::kinds::ResourceKind;

use super::{KindSpec, KindSpecs};

/// The Job's own actions.
pub const ACTIONS: &[CommandId] = &[CommandId::JOB_SUSPEND, CommandId::JOB_RESUME];

/// Where "Suspend" sits in a Job's menu: after "View Logs" (100).
pub const SUSPEND_ORDER: u16 = 150;
/// Where "Resume" sits: right after "Suspend".
pub const RESUME_ORDER: u16 = 160;

/// Where the Jobs entry sits in the Workloads section (after the ReplicationControllers).
const SIDEBAR_RANK: u32 = 70;

/// The Job kind: shared columns, the generic detail, its two actions.
pub fn spec() -> KindSpec {
    KindSpec::new(
        KindMeta::new(
            Gvk::new("batch", "v1", "Job"),
            "jobs",
            "Jobs",
            Scope::Namespaced,
        )
        .in_section(Section::WORKLOADS, SIDEBAR_RANK)
        .with_actions(ACTIONS.iter().copied()),
    )
}

/// Registers the Job kind.
pub fn init(cx: &mut App) {
    if let Err(error) = KindSpecs::register(cx, spec()) {
        tracing::error!(%error, "the Job kind was not registered");
    }
}

/// Whether `kind` is the Job kind.
pub(crate) fn is_job(kind: &ResourceKind) -> bool {
    &*kind.gvk.group == "batch" && &*kind.gvk.kind == "Job"
}

/// The row actions of Jobs: "Suspend" and "Resume", one object at a time.
pub fn row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::JOB_SUSPEND, |target| Command::JobSuspend {
            target: target.clone(),
        })
        .label("Suspend")
        .kinds(KindFilter::Matching(is_job))
        .order(SUSPEND_ORDER),
        RowActionSpec::new(CommandId::JOB_RESUME, |target| Command::JobResume {
            target: target.clone(),
        })
        .label("Resume")
        .kinds(KindFilter::Matching(is_job))
        .order(RESUME_ORDER),
    ]
}
