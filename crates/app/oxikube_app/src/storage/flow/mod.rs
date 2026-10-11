//! The flows behind the two storage dialogs: read what the dialog shows, check what the user
//! typed, and send the command through the bus.
//!
//! Like [`DeleteFlow`](crate::DeleteFlow) they are plain async Rust over the
//! [`CommandBus`](crate::CommandBus): the view spawns the future on the Tokio bridge
//! (`oxikube_runtime::spawn_kube`) and draws the answer. Nothing here writes to a cluster except
//! through the guard: the flow dispatches the command as the user, the guard answers with a
//! confirmation request, and the flow answers it on behalf of the dialog's confirm button (the
//! dialog shows the same summary the guard would).
//!
//! | File | Holds |
//! |---|---|
//! | `confirm` | the shared dispatch that answers a simple confirmation |
//! | `resize` | [`ResizeFlow`]: open (the claim and its class), check, run |
//! | `default_class` | [`DefaultClassFlow`]: open (the classes), run |

mod confirm;
mod default_class;
mod resize;

pub use default_class::{DefaultClassContext, DefaultClassFlow, DefaultClassReport};
pub use resize::{ResizeContext, ResizeFlow, ResizeReport};
