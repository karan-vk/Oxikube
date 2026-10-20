//! The start dialog (E15-S04): remote port, local port (Auto or fixed), bind address, name and a
//! favourite checkbox, opened from a Pod's container ports, a Service's ports, `shift-f` on a pod
//! row and the palette.
//!
//! | Module | What it holds |
//! |---|---|
//! | `ports` | [`PortChoice`] and [`ports_of`]: the target's TCP ports, read from the loaded object |
//! | `form` | [`StartForm`]: the fields and their validation (pure) |
//! | `problem` | [`StartProblem`] and [`Fix`]: a failed start as a message, a suggestion and a one-click fix |
//! | `prefs` | [`LastUsed`]: the last bind address and Auto/fixed choice, in memory |
//! | `view` / `render` | [`StartDialog`]: the modal; it starts the forward through `portforward::Start` |
//! | `flow` | [`StartFlow`]: opens the dialog in a cluster tab's workspace, or says why not |

mod flow;
mod form;
mod ports;
mod prefs;
mod problem;
mod render;
mod submit;
mod view;

#[cfg(test)]
mod tests;

pub use flow::{ForwardRequest, StartFlow};
pub use form::{FIRST_UNPRIVILEGED_PORT, FormErrors, FormInput, StartForm};
pub use ports::{PortChoice, ports_of};
pub use prefs::LastUsed;
pub use problem::{Fix, StartProblem};
pub use view::{Stage, StartDialog};
