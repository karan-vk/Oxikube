//! The dedicated thread that owns the SQLite connection.
//!
//! rusqlite is blocking and its `Connection` is not `Sync`, so every call is a closure sent to
//! one named thread; the caller awaits a oneshot, which keeps the UI thread free (non-negotiable
//! 7, ADR 0010). The thread runs jobs in submission order, so a write submitted before a read is
//! visible to it.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::mpsc,
    thread::JoinHandle,
};

use futures::channel::oneshot;
use oxikube_domain::{OxiError, OxiResult};
use rusqlite::Connection;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// Handle to the connection thread. Dropping it lets queued jobs finish, then joins the thread.
pub(crate) struct Worker {
    jobs: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Starts the thread, which first runs `init` to produce the connection (so opening the
    /// file, the integrity check and the migrations also happen off the caller's thread) and
    /// then serves jobs. `init`'s result comes back through the returned receiver.
    pub(crate) fn spawn<T: Send + 'static>(
        init: impl FnOnce() -> OxiResult<(Connection, T)> + Send + 'static,
    ) -> OxiResult<(Self, oneshot::Receiver<OxiResult<T>>)> {
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = oneshot::channel();
        let thread = std::thread::Builder::new()
            .name("oxikube-state".into())
            .spawn(move || {
                let mut conn = match init() {
                    Ok((conn, extra)) => {
                        let _ = ready_tx.send(Ok(extra));
                        conn
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                while let Ok(job) = jobs_rx.recv() {
                    // A panicking job must not take the store down with it.
                    if catch_unwind(AssertUnwindSafe(|| job(&mut conn))).is_err() {
                        tracing::error!("a state database job panicked");
                    }
                }
            })
            .map_err(|e| OxiError::internal("cannot start the state thread").with_source(e))?;
        Ok((
            Self {
                jobs: Some(jobs_tx),
                thread: Some(thread),
            },
            ready_rx,
        ))
    }

    /// Runs `f` on the connection thread and returns its result.
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> OxiResult<T> + Send + 'static,
    ) -> OxiResult<T> {
        let (tx, rx) = oneshot::channel();
        let job: Job = Box::new(move |conn| {
            let _ = tx.send(f(conn));
        });
        let stopped = || OxiError::internal("the state database thread has stopped");
        self.jobs
            .as_ref()
            .ok_or_else(stopped)?
            .send(job)
            .map_err(|_| stopped())?;
        rx.await.map_err(|_| stopped())?
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the channel ends the loop once queued jobs are done; joining makes a quit
        // flush every accepted write (and lets tests delete their temp dir safely).
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
