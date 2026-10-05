//! Unit tests for the log data plane. The reconnect logic runs against a scripted
//! [`LogSource`](super::source::LogSource) on tokio's paused clock, so overlap, backoff and
//! flush timing are exact and nothing sleeps; `kube_api` checks the kube-rs side against a
//! fake API server.

mod batching;
mod dedup;
mod fake;
mod fanin;
mod kube_api;
mod licence;
mod lines;
mod reconnect;
mod single;
