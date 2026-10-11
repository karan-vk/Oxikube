//! "Create secret" (E12-S11): `secret::Create` opens [`CreateSecretDialog`] in the cluster's tab;
//! a valid Secret opens in a manifest editor with its values as text, to be reviewed and applied
//! through `resource::Apply` (risk low, blocked read-only, the dry-run preview hides the values).
//!
//! | File | Holds |
//! |---|---|
//! | `dialog` | [`CreateSecretDialog`]: the fields of an Opaque, Docker registry or TLS Secret, checked through `oxikube_app::SecretService`, cleared when it closes |
//! | `view` | drawing the dialog |
//! | `open` | [`EditorViews`](super::EditorViews)`::create_secret`: the dialog in the workspace, and the editor tab its result opens |

mod dialog;
mod open;
mod view;

pub use dialog::{CreateSecretDialog, SecretKind, SecretSink};
