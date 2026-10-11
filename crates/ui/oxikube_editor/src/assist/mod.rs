//! Hover docs and completion from the cluster's schemas (E10-S05).
//!
//! The pure core: [`hover_at`] and [`complete_at`] take the spanned model of a buffer
//! ([`yaml::ParseResult`](crate::yaml::ParseResult)), a way to find the schema of a kind and a
//! byte offset, and answer from the already-flattened [`JsonSchema`](oxikube_domain::schema::JsonSchema)
//! in O(depth) beyond the parse. They know nothing of gpui, `SchemaPort` or the UI thread, so
//! they are unit-tested without a window; [`ManifestAssist`] runs them on the background
//! executor for the editor (`oxikube_ui::editor::EditorAssist`).
//!
//! | Module | What |
//! |---|---|
//! | `buffer` | [`Buffer`]: the text, parsed only if completion needs it |
//! | `lookup` | the schema at a JSON path: properties, `items`, map values; open and unknown told apart |
//! | `describe` | type labels (`string (enum)`, `array of object`), YAML scalars, field docs |
//! | `hover` | [`hover_at`] and [`HoverDoc`] |
//! | `line` | the cursor's line read lexically: key or value, comment, block scalar |
//! | `complete` | [`complete_at`] and [`CompletionItem`] |
//! | `values` | value completion: enum members, booleans |
//! | `snippets` | the container and volume blocks |
//! | `provider` | [`ManifestAssist`]: the editor's `EditorAssist` over the known schemas |
//!
//! Where the schema is **open** (a free-form object, `x-kubernetes-preserve-unknown-fields`, a
//! subtree the flattening cut off) nothing is offered and nothing is documented, because the
//! schema does not know. A schema-valued `additionalProperties` (a map such as `labels`) offers
//! no keys; its values are documented with the value schema.

mod buffer;
mod complete;
mod describe;
mod hover;
mod line;
mod lookup;
mod provider;
mod snippets;
mod values;

pub use buffer::Buffer;
pub use complete::{CompletionItem, CompletionKind, can_complete, complete_at};
pub use hover::{HoverDoc, hover_at};
pub use provider::ManifestAssist;
