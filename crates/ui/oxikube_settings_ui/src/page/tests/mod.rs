//! Tests of the settings page: the model built from the schema, the search, the badge's words,
//! and the page in a workspace window over a settings store installed from a temporary
//! directory (`init_with_dir`, no watcher).

mod model;
mod scenarios;
mod search;
mod view;

pub(super) use super::harness::Harness;
