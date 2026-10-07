//! Saving a log view's lines to a file (E08-S06): the dialog that says what would be written, and
//! the request it hands the view.
//!
//! `logs::Save` (`ctrl-s` saves everything the buffer holds, `ctrl-shift-s` the lines on screen)
//! opens a [`SaveDialog`] in the cluster tab's modal layer. It states which lines the chosen scope
//! covers and how many, offers the timestamp and pod prefix toggles, and says so when the buffer
//! dropped older lines. Only its "Choose file…" button goes on: the platform's save panel asks
//! where to write, and cancelling it writes nothing. The write itself ([`LogView::save_chosen`]
//! in `view/save.rs`) streams through the `FsPort` on the Tokio bridge.
//!
//! | File | Holds |
//! |---|---|
//! | `dialog` | [`SaveDialog`], [`SaveOffer`], [`SaveRequest`] |
//! | `name` | [`suggested_file_name`] |
//!
//! [`LogView::save_chosen`]: crate::LogView::save_chosen

mod dialog;
mod name;

pub use dialog::{SaveDialog, SaveOffer, SaveRequest};
pub use name::suggested_file_name;
