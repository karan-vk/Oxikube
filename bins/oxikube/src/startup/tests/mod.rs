//! The real init order under `#[gpui::test]`: no OS threads (deterministic runtime, in-memory or
//! one-shot config, no watchers), testkit fakes for the ports.

mod order;
mod quit;
mod state_db;
