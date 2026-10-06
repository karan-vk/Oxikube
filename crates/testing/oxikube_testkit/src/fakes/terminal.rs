//! Terminal fakes: [`FakeTerminalBackend`] and [`FakeExecPort`].
//!
//! [`FakeTerminalBackend`] is a scripted `TerminalBackend`: by default it echoes every
//! write back as output, a [`Timeline`] adds output, errors and an exit at chosen offsets
//! on a [`FakeClockPort`], and every write, resize and kill is recorded. It is a cheap
//! `Clone` handle: the test keeps one clone to emit events and inspect calls, the app
//! under test gets another as a `Box<dyn TerminalBackend>`.
//!
//! [`FakeExecPort`] is the fake `ExecPort`: each `exec` / `attach` / `create_debug_container`
//! / `node_shell` records the requested descriptor and hands out the next queued
//! [`FakeTerminalBackend`], or a fresh echoing one when nothing is queued.
//!
//! How a service drives the fake (the compiled twin is `tests/terminal_backend.rs`):
//!
//! ```text
//! let port = FakeExecPort::new();
//! let remote = FakeTerminalBackend::echo();           // keep a handle
//! port.script().exec.push_ok(remote.clone());
//! let backend = port.exec(&target).await?;            // Box<dyn TerminalBackend>
//! let mut events = backend.output_stream();
//! backend.write(b"ls\n").await?;
//! assert!(matches!(events.next().await, Some(BackendEvent::Output(b)) if &b[..] == b"ls\n"));
//! remote.exit(ExitStatus::success());                 // the stream ends after Exited
//! ```

mod backend;
mod port;

pub use backend::{FakeTerminalBackend, TerminalCall};
pub use port::{ExecPortCall, ExecPortScripts, FakeExecPort};

#[cfg(test)]
mod tests;
