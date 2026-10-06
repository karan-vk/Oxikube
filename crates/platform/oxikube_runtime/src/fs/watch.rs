//! `FsPort::watch` on `notify`.

use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::channel::mpsc;
use futures::stream::{BoxStream, Stream, StreamExt as _};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use oxikube_ports::{FsEvent, FsEventKind};

/// The event stream, owning the watcher: dropping the stream drops the watcher and stops it.
struct Watch {
    events: mpsc::UnboundedReceiver<FsEvent>,
    _watcher: Option<RecommendedWatcher>,
}

impl Stream for Watch {
    type Item = FsEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<FsEvent>> {
        self.events.poll_next_unpin(cx)
    }
}

/// Watches `path` (recursively for a directory). A path that cannot be watched gives a stream
/// that never yields, and the reason is logged: callers also reload on their own schedule.
pub(super) fn watch(path: PathBuf) -> BoxStream<'static, FsEvent> {
    let (tx, events) = mpsc::unbounded();
    let handler = move |event: notify::Result<Event>| {
        // Runs on notify's own thread: only a channel send.
        let Ok(event) = event else { return };
        let kind = match event.kind {
            EventKind::Create(_) => FsEventKind::Created,
            EventKind::Modify(_) => FsEventKind::Modified,
            EventKind::Remove(_) => FsEventKind::Removed,
            _ => return,
        };
        for path in event.paths {
            let _ = tx.unbounded_send(FsEvent { path, kind });
        }
    };
    let watcher = notify::recommended_watcher(handler).and_then(|mut watcher| {
        watcher.watch(&path, RecursiveMode::Recursive)?;
        Ok(watcher)
    });
    let watcher = match watcher {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "could not watch a path");
            None
        }
    };
    Watch {
        events,
        _watcher: watcher,
    }
    .boxed()
}
