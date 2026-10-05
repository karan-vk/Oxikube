//! kube-runtime `watcher::Error` to [`OxiError`].
//!
//! The watcher retries every error itself (with backoff), so the question here is only
//! whether the feed should keep going. Transient failures (network, timeouts, a stream
//! that broke mid-watch, HTTP 410 Gone) are retryable and the feed continues. A refusal
//! the next attempt will meet again (403, 404, a kind that cannot be watched) is not, and
//! ends the feed after it is reported, as the `WatchFeed` contract says.

use kube::core::Status;
use kube::runtime::watcher;
use oxikube_domain::OxiError;

use crate::auth::classify;
use crate::resources::{ListExpired, list_error};

/// The domain error for a watcher failure.
pub(crate) fn feed_error(err: &watcher::Error) -> OxiError {
    use watcher::Error as E;
    match err {
        E::InitialListFailed(e) | E::WatchStartFailed(e) => list_error(e),
        // An API status on the watch: an `ERROR` event, or the watch request itself refused
        // (kube reads the response body as the event stream, so a 403 arrives here too).
        E::WatchError(status) | E::WatchFailed(kube::Error::Api(status)) => status_error(status),
        // The watch was open: a broken stream or an undecodable event. The watcher resumes
        // from the last resource version, so this is always worth retrying.
        E::WatchFailed(e) => classify(e).with_retryable(true),
        E::NoResourceVersion => OxiError::unsupported(
            "the server sent no resourceVersion, so this kind cannot be watched",
        ),
    }
}

fn status_error(status: &Status) -> OxiError {
    if status.code == 410 {
        return OxiError::conflict(
            "the watch's resource version expired (HTTP 410 Gone); relisting",
        )
        .with_source(ListExpired)
        .with_retryable(true);
    }
    let err = classify(&kube::Error::Api(Box::new(status.clone())));
    // 5xx and 429 on a watch are server hiccups.
    let transient = status.code >= 500 || status.code == 429;
    let retryable = err.is_retryable() || transient;
    err.with_retryable(retryable)
}

/// Whether the server refused a streaming list (`sendInitialEvents`), as an API server
/// with the WatchList feature gate off does (HTTP 400 or 422 on the initial watch).
pub(crate) fn streaming_rejected(err: &watcher::Error) -> bool {
    use watcher::Error as E;
    matches!(
        err,
        E::WatchStartFailed(kube::Error::Api(status)) | E::WatchFailed(kube::Error::Api(status))
            if status.code == 400 || status.code == 422
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::is_list_expired;
    use oxikube_domain::ErrorKind;

    fn status(code: u16) -> Box<Status> {
        Box::new(Status::failure("boom", "Reason").with_code(code))
    }

    #[test]
    fn gone_is_a_retryable_list_expired_conflict() {
        let err = feed_error(&watcher::Error::WatchError(status(410)));
        assert!(is_list_expired(&err));
        assert!(err.is_retryable());
    }

    #[test]
    fn forbidden_ends_the_feed_and_server_errors_do_not() {
        let forbidden = feed_error(&watcher::Error::InitialListFailed(kube::Error::Api(
            status(403),
        )));
        assert_eq!(forbidden.kind(), ErrorKind::Forbidden);
        assert!(!forbidden.is_retryable());
        assert!(feed_error(&watcher::Error::WatchError(status(500))).is_retryable());
        assert!(!feed_error(&watcher::Error::NoResourceVersion).is_retryable());
        let refused_watch = watcher::Error::WatchFailed(kube::Error::Api(status(403)));
        assert!(
            !feed_error(&refused_watch).is_retryable(),
            "a refused watch is final"
        );
        let broken = watcher::Error::WatchFailed(kube::Error::LinesCodecMaxLineLengthExceeded);
        assert!(
            feed_error(&broken).is_retryable(),
            "a broken stream is retried"
        );
    }

    #[test]
    fn a_rejected_streaming_list_is_recognised() {
        let rejected = watcher::Error::WatchStartFailed(kube::Error::Api(status(422)));
        assert!(streaming_rejected(&rejected));
        let in_stream = watcher::Error::WatchFailed(kube::Error::Api(status(400)));
        assert!(streaming_rejected(&in_stream));
        let forbidden = watcher::Error::WatchStartFailed(kube::Error::Api(status(403)));
        assert!(!streaming_rejected(&forbidden));
    }
}
