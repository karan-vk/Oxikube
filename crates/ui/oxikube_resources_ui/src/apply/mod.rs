//! The apply panel (E20-S05): pick a file, a directory or a kustomize directory, review the plan
//! with the server's dry run of every object, apply it, and read what happened to each object.
//!
//! ```text
//! apply::Directory / apply::Kustomize ─(palette, an agent)─> ApplyViews::open ─> ApplyView in the
//!   cluster tab's workspace (one per tab, reused)
//! Load    ApplyBackend::load (FsPort walk, or the kustomize build; off the UI thread)
//!           path, objects, files, parse errors with file and line, kustomize tool and stderr
//! Review  ApplyBackend::plan (kind order, namespace) ─> dry_run: one result per object as it is
//!           decided; read-only clusters show the lock and disable Apply with the reason
//! Result  Apply ─> confirmation bar (the dry run's counts) ─> ApplyBackend::apply: one guarded
//!           resource::ApplyPlan dispatch, results streamed; Cancel stops between objects
//! ```
//!
//! | Module | What |
//! |---|---|
//! | `model` | [`Model`]: stage, activity, rows, filter, expansion, copy as text; no window |
//! | `backend` | [`ApplyBackend`]: what the panel needs from outside |
//! | `service` | [`ServiceBackend`]: the backend over `ApplyService`, `FsPort`, kustomize and the state port |
//! | `view` / `flow` | [`ApplyView`]: the entity, and load / plan / dry run / apply / cancel / re-run / open in editor |
//! | `render` | the header, the Load view, the virtualised table, the footer |
//! | `open` | the `apply::Directory` / `apply::Kustomize` handlers and [`ApplyViews`], which open the panel in the cluster tab |
//! | `keys` | the `ApplyResults` key context's actions |
//!
//! # Safety
//!
//! The panel never writes to a cluster itself. The dry run and the apply are `ApplyService`
//! dispatches of `resource::ApplyPlan`: the guard refuses them on a read-only cluster, asks one
//! confirmation (answered by the panel's bar, after it showed the dry run's counts), and audits
//! one record per object. The commands that open the panel change nothing. Objects may be
//! Secrets: the model keeps names, statuses and the server's messages, never an object's content,
//! and nothing is logged.

mod backend;
mod document;
mod flow;
mod keys;
mod model;
mod open;
mod render;
mod service;
mod source;
mod view;

#[cfg(test)]
mod tests;

pub use backend::{ApplyBackend, Loaded, ResultStream};
pub use document::{EditorDocument, EditorOpener};
pub use keys::{
    OpenInEditor, SelectFirst, SelectLast, SelectNext, SelectPrevious, ToggleErrorsOnly,
    ToggleExpand,
};
pub use model::{Activity, Line, LoadSummary, Model, Row, Stage, details};
pub use open::{ApplyRequest, ApplyRequestSink, ApplyViews, ApplyViewsDeps, register_commands};
pub use render::{ROW_HEIGHT, status_look};
pub use service::ServiceBackend;
pub use source::{ApplySource, SourceKind};
pub use view::{ApplyDeps, ApplyView, ITEM_KEY};
