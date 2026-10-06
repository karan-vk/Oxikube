//! Delete (E07-S08): the `resource::Delete` handler and the flow that runs it for a selection.
//!
//! | File | Holds |
//! |---|---|
//! | `handler` | [`register_commands`]: the `resource::Delete` handler, the only code that calls `ResourceWriter::delete` for it, through the guard's `Mutation` |
//! | `plan` | [`DeleteFlow::plan`] and [`DeletePlan`]: what the guard will ask, before anything is sent |
//! | `run` | [`DeleteFlow::run`], [`DeleteReport`] and the per-object [`ItemResult`]s |

mod handler;
mod plan;
mod run;

pub use handler::register_commands;
pub use plan::{DeleteError, DeleteFlow, DeletePlan, PlannedDelete};
pub use run::{DeleteReport, ItemResult, ItemStatus};
