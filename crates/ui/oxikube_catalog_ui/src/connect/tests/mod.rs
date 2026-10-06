//! Tests of the connect lifecycle UI: the pure model (`model`), and `#[gpui::test]`s of the views
//! in a cluster tab over testkit fakes (`states`, `commands`, `redaction`).

mod commands;
mod fixture;
mod model;
mod redaction;
mod states;

pub(super) use fixture::id;
