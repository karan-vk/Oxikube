//! `#[gpui::test]`s of the log view over testkit fakes: a connected cluster whose `LogPort` replays
//! scripted timelines on a fake clock, the app's `LogService` on the deterministic runtime, and a
//! dispatcher that does what the bus does. No cluster, no threads, no sleeping.

mod containers;
mod fixture;
mod keys;
mod scroll;
mod stream;
