//! Unit tests for the write path: a scripted in-process API server behind a real
//! `kube::Client`, with legacy discovery serving kinds that allow writes.

mod errors;
mod harness;
mod ops;
mod patches;
