//! Unit tests for the resource data plane: a scripted in-process API server behind a real
//! `kube::Client`, with legacy discovery serving a few kinds.

mod conversion;
mod errors;
mod harness;
mod list;
mod paging;
mod params;
mod typed_dynamic;
