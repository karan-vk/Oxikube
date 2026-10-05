//! Unit tests for the Table API feed: recorded Table fixtures behind the scripted
//! in-process API server (`fake_api`), with legacy discovery serving pods, the `Widget` CRD
//! and a list-only aggregated kind.

mod diff;
mod feed;
mod harness;
mod parse;
mod perf;
mod relist;
mod request;
