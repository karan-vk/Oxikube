//! What the follower does between responses: whether to open again, the request that
//! continues the log, the previous instance's tail after a restart, and failed opens.

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::{ErrorKind, OxiError};
use oxikube_ports::LogSince;

use super::{End, Follower, Reopen};
use crate::logs::source::{ContainerState, OpenRequest};
use crate::logs::stream::Closed;

impl Follower {
    /// The pod after a dropped stream: whether to open again.
    pub(super) async fn should_reconnect(&mut self, clean: bool) -> bool {
        let target = &self.target;
        let pod = match self.source.pod(&target.namespace, &target.pod).await {
            Ok(Some(pod)) => pod,
            Ok(None) => return false,
            // Cannot tell; the next open decides.
            Err(_) => return true,
        };
        if self.uid.as_deref().is_some_and(|uid| uid != pod.uid) {
            return false;
        }
        self.uid.get_or_insert_with(|| pod.uid.clone());
        let Some(container) = pod.container(&target.container) else {
            return false;
        };
        let finished = matches!(container.state, ContainerState::Terminated { .. })
            && !pod.will_restart(container);
        if clean && finished {
            return false;
        }
        self.replay_previous |= self.restarts.map_or(container.restart_count > 0, |seen| {
            container.restart_count > seen
        });
        self.restarts = Some(container.restart_count);
        true
    }

    pub(super) async fn reopen(&mut self, failures: &mut u32) -> Reopen {
        if std::mem::take(&mut self.replay_previous) && self.read_previous().await.is_err() {
            return Reopen::Stop;
        }
        let request = self.resume_request();
        if self.dedup.newest().is_some() {
            self.dedup.begin_replay();
        }
        match self
            .source
            .open(&self.target.namespace, &self.target.pod, &request)
            .await
        {
            Ok(reader) => {
                *failures = 0;
                Reopen::Open(reader)
            }
            Err(err) => self.open_failed(err, failures).await,
        }
    }

    /// The request that continues the log: from the overlap before the last line seen, or the
    /// caller's own options when nothing was delivered yet.
    fn resume_request(&self) -> OpenRequest {
        match self.dedup.newest() {
            Some(newest) => {
                OpenRequest::resume(&self.target.container, false, self.overlap_start(newest))
            }
            None => OpenRequest::first(&self.target.container, &self.options),
        }
    }

    fn overlap_start(&self, newest: Timestamp) -> LogSince {
        let overlap = SignedDuration::from_secs(self.config.overlap_secs());
        LogSince::Time(newest.checked_sub(overlap).unwrap_or(newest))
    }

    /// Delivers what the previous container instance logged after the overlap start (kdash
    /// `fetch_previous_logs`, used here to close the gap a restart leaves).
    async fn read_previous(&mut self) -> Result<(), Closed> {
        let Some(newest) = self.dedup.newest() else {
            return Ok(());
        };
        let request = OpenRequest::resume(&self.target.container, true, self.overlap_start(newest));
        self.dedup.begin_replay();
        match self
            .source
            .open(&self.target.namespace, &self.target.pod, &request)
            .await
        {
            Ok(reader) => match self.pump(reader).await {
                End::Closed => Err(Closed),
                End::Eof | End::Failed(_) => Ok(()),
            },
            // No previous instance (or it is not readable): nothing to fill.
            Err(err) => {
                tracing::debug!(kind = ?err.kind(), "previous container log not available");
                Ok(())
            }
        }
    }

    async fn open_failed(&mut self, err: OxiError, failures: &mut u32) -> Reopen {
        if !self.reconnect {
            // A single read has nothing to wait for: report why it could not start.
            let _ = self.sink.fail(err).await;
            return Reopen::Stop;
        }
        match err.kind() {
            // The pod (or container) is gone: the log has ended.
            ErrorKind::NotFound => return Reopen::Stop,
            ErrorKind::Auth | ErrorKind::Forbidden if !err.is_retryable() => {
                let _ = self.sink.fail(err).await;
                return Reopen::Stop;
            }
            _ => {}
        }
        let target = &self.target;
        let waiting = match self.source.pod(&target.namespace, &target.pod).await {
            Ok(None) => return Reopen::Stop,
            Ok(Some(pod)) if self.uid.as_deref().is_some_and(|uid| uid != pod.uid) => {
                return Reopen::Stop;
            }
            // A container that is starting, between restarts or without a status yet has no
            // log to open; waiting for it is not a failure.
            Ok(Some(pod)) => pod
                .container(&target.container)
                .is_some_and(|c| pod.awaiting_start(c)),
            Err(_) => false,
        };
        if !waiting {
            *failures += 1;
            if *failures >= self.config.max_open_failures {
                let _ = self.sink.fail(err).await;
                return Reopen::Stop;
            }
        }
        Reopen::Wait
    }
}
