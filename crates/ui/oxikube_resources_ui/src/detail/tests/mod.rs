//! Tests of the detail drawer: the pure models live next to them (`model/tests.rs`,
//! `events/tests.rs`); these are `#[gpui::test]`s over testkit fakes: a fake connector, a real
//! session manager and store, a recording dispatcher, and the real `ResourceViews` opening the
//! drawer in a cluster tab. No cluster, no disk, no threads.

mod describe;
mod fixture;
mod meta;
mod open;
mod pin;
mod schema;
mod secret;
mod states;
mod yaml;
