//! Unit tests for exec, attach, node shells and debug containers: the stream adapters and the
//! session against in-memory pipes, the open path against the in-process fake API, and the
//! node-shell and debug flows against a scripted pod API and the testkit's `FakeExecStreamPort`.
//! The websocket itself is covered on kind by `tests/exec_*.rs`.

mod debug;
mod fakes;
mod kube_pods;
mod node_shell;
mod open;
mod pipes;
mod session;
mod terminal;
mod wait;
