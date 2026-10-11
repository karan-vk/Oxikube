//! CronJobs (E12-S08): the kind's spec and its row actions, trigger now, suspend and resume.
//!
//! The columns (Schedule, Timezone, Suspend, Active, Last schedule, Age) are the shared core
//! catalogue's; the actions are the `cronjob::*` commands `oxikube_app::workloads` handles. The
//! `t` key triggers and `s` suspends or resumes (k9s), both scoped to CronJob tables in the
//! default keymaps, and both act on the cursor row.

use gpui::App;
use oxikube_app::actions::KindFilter;
use oxikube_app::{KindMeta, RowActionSpec, Section};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::kinds::ResourceKind;

use super::{KindSpec, KindSpecs};

/// The CronJob's own actions.
pub const ACTIONS: &[CommandId] = &[
    CommandId::CRONJOB_TRIGGER,
    CommandId::CRONJOB_SUSPEND,
    CommandId::CRONJOB_RESUME,
];

/// Where "Trigger" sits in a CronJob's menu: after "View Logs" and the shells (100 to 130).
pub const TRIGGER_ORDER: u16 = 140;
/// Where "Suspend" sits: right after "Trigger".
pub const SUSPEND_ORDER: u16 = 150;
/// Where "Resume" sits: right after "Suspend".
pub const RESUME_ORDER: u16 = 160;

/// Where the CronJobs entry sits in the Workloads section (after the Jobs).
const SIDEBAR_RANK: u32 = 80;

/// The CronJob kind: shared columns, the generic detail, its three actions.
pub fn spec() -> KindSpec {
    KindSpec::new(
        KindMeta::new(
            Gvk::new("batch", "v1", "CronJob"),
            "cronjobs",
            "CronJobs",
            Scope::Namespaced,
        )
        .in_section(Section::WORKLOADS, SIDEBAR_RANK)
        .with_actions(ACTIONS.iter().copied()),
    )
}

/// Registers the CronJob kind.
pub fn init(cx: &mut App) {
    if let Err(error) = KindSpecs::register(cx, spec()) {
        tracing::error!(%error, "the CronJob kind was not registered");
    }
}

/// Whether `kind` is the CronJob kind.
pub(crate) fn is_cronjob(kind: &ResourceKind) -> bool {
    &*kind.gvk.group == "batch" && &*kind.gvk.kind == "CronJob"
}

/// The row actions of CronJobs: "Trigger", "Suspend" and "Resume", one object at a time. Each
/// mutates, so the menu greys it out on a read-only cluster and the bus asks for a confirmation
/// (risk low).
pub fn row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::CRONJOB_TRIGGER, |target| {
            Command::CronJobTrigger {
                target: target.clone(),
            }
        })
        .label("Trigger")
        .kinds(KindFilter::Matching(is_cronjob))
        .order(TRIGGER_ORDER),
        RowActionSpec::new(CommandId::CRONJOB_SUSPEND, |target| {
            Command::CronJobSuspend {
                target: target.clone(),
            }
        })
        .label("Suspend")
        .kinds(KindFilter::Matching(is_cronjob))
        .order(SUSPEND_ORDER),
        RowActionSpec::new(CommandId::CRONJOB_RESUME, |target| Command::CronJobResume {
            target: target.clone(),
        })
        .label("Resume")
        .kinds(KindFilter::Matching(is_cronjob))
        .order(RESUME_ORDER),
    ]
}
