//! Manifest builders: the objects a kubectl-equivalent action creates, as pure functions of the
//! objects they start from (no clock, no randomness, no I/O). Builders that carry ported code
//! keep its licence header.
//!
//! | File | Holds |
//! |---|---|
//! | `cronjob` | [`job_from_cronjob`]: the Job `kubectl create job --from=cronjob/x` builds (kdash, MIT) |
//! | `suspend` | [`suspend_patch`]: the merge patch that suspends or resumes a Job or CronJob |

mod cronjob;
mod suspend;

pub use cronjob::{INSTANTIATE_ANNOTATION, job_from_cronjob};
pub use suspend::suspend_patch;
