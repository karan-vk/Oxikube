//! Unit tests for discovery. `fake` is an in-process API server that serves the documents in
//! `tests/fixtures/discovery/` through a real `kube::Client`.

mod convert;
mod debounce;
mod fake;
mod fetch;
mod registry;
mod resolve;
mod serves_group;
