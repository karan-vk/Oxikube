//! `#[gpui::test]`s of the log view over testkit fakes: a connected cluster whose `LogPort` replays
//! scripted timelines on a fake clock, the app's `LogService` on the deterministic runtime, and a
//! dispatcher that does what the bus does. No cluster, no threads, no sleeping.

mod agent;
mod aggregate;
mod containers;
mod fixture;
mod json;
mod keymap;
mod keys;
mod local;
mod open;
mod save;
mod scroll;
mod search;
mod settings;
mod stream;
