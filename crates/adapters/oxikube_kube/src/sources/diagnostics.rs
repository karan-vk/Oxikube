//! The loader's findings as the port's [`SourceDiagnostic`]s, and the feed that reports changes
//! to them (E03-F439).
//!
//! The loader's [`Diagnostic`] stays the adapter's own type: it is richer (it carries the winner
//! and loser of a shadowed context, the tier that was picked) and tests assert on it. The port
//! gets a flat, display-ready view. Both are free of file contents: the loader never keeps
//! parser messages, and the mapping only reads the loader's fixed wording.

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use oxikube_ports::{DiagnosticSeverity, SourceDiagnostic};

use super::{Inner, pasted};
use crate::kubeconfig::{Diagnostic, Severity};

/// The port's view of one loader diagnostic.
///
/// The path is the file or directory the finding is about; a pasted kubeconfig has no file, so
/// its stand-in path is dropped (the message still names it, which is an id, not text).
pub(super) fn to_port(diagnostic: &Diagnostic) -> SourceDiagnostic {
    let severity = match diagnostic.severity() {
        Severity::Info => DiagnosticSeverity::Info,
        Severity::Warning => DiagnosticSeverity::Warning,
    };
    let path = match diagnostic {
        Diagnostic::MissingFile { path }
        | Diagnostic::BlankFile { path }
        | Diagnostic::Unreadable { path, .. }
        | Diagnostic::Unparsable { path }
        | Diagnostic::Incompatible { path, .. }
        | Diagnostic::DirectoryNotWatched { path }
        | Diagnostic::DuplicateContext { shadowed: path, .. } => Some(path),
        _ => None,
    };
    let mut out = SourceDiagnostic::new(severity, diagnostic.to_string());
    if let Some(path) = path.filter(|p| !pasted::is_pseudo_path(p)) {
        out = out.with_path(path);
    }
    out
}

/// The diagnostics last seen, and the subscribers told when they change.
#[derive(Default)]
pub(super) struct DiagnosticFeed {
    last: Vec<SourceDiagnostic>,
    subscribers: Vec<mpsc::UnboundedSender<Vec<SourceDiagnostic>>>,
}

impl Inner {
    /// Everything the last load skipped or shadowed, plus directories the watcher could not
    /// register, in the loader's shape. Empty before the first load.
    pub(super) fn loader_diagnostics(&self) -> Vec<Diagnostic> {
        let mut found: Vec<Diagnostic> = self
            .current
            .read()
            .iter()
            .flat_map(|s| s.diagnostics.iter().cloned())
            .collect();
        found.extend(
            self.unwatched
                .lock()
                .iter()
                .map(|path| Diagnostic::DirectoryNotWatched { path: path.clone() }),
        );
        found
    }

    /// [`loader_diagnostics`](Self::loader_diagnostics) as the port reports them.
    pub(super) fn port_diagnostics(&self) -> Vec<SourceDiagnostic> {
        self.loader_diagnostics().iter().map(to_port).collect()
    }

    /// Records the current diagnostics and, when `notify` and they differ from the last ones,
    /// sends the full list to every subscriber. Called after a load and when the watcher's
    /// outcome changes; the feed lock makes the two agree on order.
    pub(super) fn publish_diagnostics(&self, notify: bool) {
        let mut state = self.diagnostic_feed.lock();
        let now = self.port_diagnostics();
        if state.last == now {
            return;
        }
        state.last = now.clone();
        if notify {
            state
                .subscribers
                .retain(|tx| tx.unbounded_send(now.clone()).is_ok());
        }
    }

    pub(super) fn subscribe_diagnostics(&self) -> BoxStream<'static, Vec<SourceDiagnostic>> {
        let (tx, rx) = mpsc::unbounded();
        self.diagnostic_feed.lock().subscribers.push(tx);
        rx.boxed()
    }
}
