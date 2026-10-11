//! Manifest builders: the objects a kubectl-equivalent action creates, as pure functions of the
//! objects they start from (no clock, no randomness, no I/O). Builders that carry ported code
//! keep its licence header.
//!
//! | File | Holds |
//! |---|---|
//! | `cronjob` | [`job_from_cronjob`]: the Job `kubectl create job --from=cronjob/x` builds (kdash, MIT) |
//! | `suspend` | [`suspend_patch`]: the merge patch that suspends or resumes a Job or CronJob |
//! | `restart` | [`restart_patch`] and [`restart_stamp`]: the merge patch of `kubectl rollout restart` and its timestamp (kdash, MIT) |
//! | `images` | `kubectl set image`: [`template_images`], [`plan_image_changes`] and the strategic [`set_image_patch`] |

mod cronjob;
mod images;
mod restart;
mod suspend;

pub use cronjob::{INSTANTIATE_ANNOTATION, job_from_cronjob};
pub use images::{
    ImageEdit, ImageSlot, image_problem, plan_image_changes, set_image_patch, template_images,
};
pub use restart::{RESTARTED_AT_ANNOTATION, restart_patch, restart_stamp};
pub use suspend::suspend_patch;
