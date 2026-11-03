//! The controls of the settings page (E21-S05): one control per field, chosen by the field's
//! schema type, writing the user's `settings.json` through the comment-preserving edit.
//!
//! | Schema | Control |
//! |---|---|
//! | `boolean` | a switch, written at once |
//! | `enum` / `oneOf` of `const`s | one button per value, written at once |
//! | `integer`, `number` (`minimum`, `maximum`) | a text input checked against the bounds |
//! | `string` | a text input |
//! | `string`, `format: "color"` (or a hex `pattern`) | an input with a swatch |
//! | `x-oxikube-dynamic-enum: "fonts"` | an input with the installed font families (listed in the background) |
//! | array of strings | a list editor: add, remove, reorder |
//! | anything else | a read-only summary and "edit in settings.json" |
//!
//! Behaviour shared by all of them:
//!
//! * Typing is validated first ([`validate_text`], [`validate_list`]); a value the schema does not
//!   allow shows an inline message under the control and is never written, so the file keeps the
//!   last valid value.
//! * A valid value is written after a 300 ms pause in typing (switches and choices at once) by
//!   [`update_user_settings_path`](oxikube_settings::update_user_settings_path), which edits the
//!   text in place and keeps comments. Every field has one write in flight at a time; a newer
//!   value waits for it.
//! * "Reset" appears only where the user layer (or, on the cluster tab, the cluster layer) holds a
//!   value, and removes the key so the default shows through; an empty input does the same. An
//!   empty list is written as `[]`. Editing a list rewrites that array, so comments inside the array may move.
//! * The page follows the file: when the store changes, an input that is neither focused nor
//!   waiting to be written shows the new value; the app's own writes echo the value already in the
//!   input and change nothing, so the cursor does not jump.
//!
//! | Module | Holds |
//! |---|---|
//! | `spec` | [`ControlSpec`]: the control for a schema node |
//! | `validate` | the pure checks of typed input |
//! | `state` | [`Controls`]: the input entities, errors and write bookkeeping of a page |
//! | `fonts` | the installed font families, listed once per app off the UI thread |
//! | `write` | the debounce, the one-in-flight write and reset |
//! | `list` | the list editor's rows |
//! | `render` | the control elements and the inline error |

mod fonts;
mod list;
mod render;
mod spec;
mod state;
mod validate;
mod write;

pub use spec::{ControlSpec, NumberBounds};
pub use state::Controls;
pub use validate::{FieldError, validate_list, validate_text};

use std::time::Duration;

/// How long typing must pause before the value is written.
pub const WRITE_DEBOUNCE: Duration = Duration::from_millis(300);

#[cfg(test)]
mod tests;
