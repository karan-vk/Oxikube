//! The key recorder (E21-S08): a modal that records the keys of a binding for the keymap
//! editor's selected action, lists the bindings in force they clash with, and saves the binding
//! to the user's `keymap.json`.
//!
//! Recording uses GPUI's keystroke interception (`App::intercept_keystrokes`): while the recorder
//! records, every keystroke of its window is taken before the keymap sees it, so no binding fires,
//! Escape and Enter included (they are legitimate bindings). The recorder therefore has explicit
//! Record / Stop, Clear, Cancel and Save buttons; a sequence (`g g`, `cmd-k cmd-s`) ends after
//! [`IDLE_TIMEOUT`] without a key, or at the fourth keystroke. A modifier pressed alone is not a
//! binding and is skipped. The keys are written with the platform's modifier names (`cmd-shift-k`
//! on macOS, `ctrl-shift-k` elsewhere), as `oxikube_keymap::edit::keystroke_text` spells them.
//!
//! Each change looks for conflicts across the context chain at once
//! (`oxikube_keymap::edit::ContextModel`: same keys or a prefix, in contexts that can be active
//! together, in any layer), and validates the edit after [`CHECK_DEBOUNCE`] on the background
//! executor with the one keymap validator; Save stays disabled while the edit would add an error.
//! The user answers a conflict with "Keep both" (the later layer wins, the user's), "Replace them"
//! (the others are deleted, or unbound with `null` when they are defaults) or Cancel.
//!
//! | Module | Holds |
//! |---|---|
//! | `model` | [`RecorderModel`]: the keys, context, conflicts, choice, verdict and the payload, as data |
//! | `view` | [`KeybindRecorder`]: the modal entity, its interception and its two timers |
//! | `render` | drawing it |

mod model;
mod render;
mod view;

pub use model::{ConflictChoice, MAX_KEYSTROKES, RecorderModel, Verdict};
pub use render::RECORDER_KEY_CONTEXT;
pub use view::{CHECK_DEBOUNCE, IDLE_TIMEOUT, KeybindRecorder};
