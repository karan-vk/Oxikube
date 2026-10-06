//! `tracing` setup: rolling log files, optional stderr, a reloadable filter, redaction everywhere.
//!
//! [`init`] installs the global subscriber (and the `log` -> `tracing` bridge, so GPUI's and every
//! dependency's `log::` records land in the same file). [`build`] makes the same subscriber
//! without installing it, for tests and tools.
//!
//! The subscriber is a [`Registry`] with, in order:
//!
//! 1. a [`reload`]able [`EnvFilter`] (see [`LogHandle`]),
//! 2. the redacting fmt layer writing to a rolling file (daily, [`LogConfig::max_files`] kept)
//!    through a non-blocking writer: the calling thread only enqueues the line, a worker thread
//!    does the write, so logging never blocks the UI thread. When the queue is full lines are
//!    dropped, not waited for.
//! 3. optionally the same layer on stderr.
//!
//! Both layers are [`redacting_layer`]s, so every event is scrubbed by name and by pattern
//! before it reaches a sink (see the crate docs).

use std::path::PathBuf;
use std::sync::Arc;

use tracing::Dispatch;
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Registry, reload};

use crate::{DEFAULT_DIRECTIVES, redacting_layer};

/// Lines the non-blocking writer buffers before it starts dropping them.
const BUFFERED_LINES: usize = 8_192;

/// Where and how to log.
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// Directory of the rolling log files (created when missing).
    pub dir: PathBuf,
    /// File name prefix: files are `<prefix>.<YYYY-MM-DD>.log`.
    pub file_prefix: String,
    /// Log files kept; older ones are deleted when a new day's file is created.
    pub max_files: usize,
    /// Initial filter directives (the `log.filter` setting, `RUST_LOG` syntax). `None`, or text
    /// that does not parse, means [`DEFAULT_DIRECTIVES`].
    pub directives: Option<String>,
    /// Whether a valid `RUST_LOG` overrides `directives` (a developer's escape hatch). While it
    /// does, the `log.filter` setting is not applied (see [`SetOutcome::PinnedByRustLog`]).
    pub honour_rust_log: bool,
    /// Also write to stderr.
    pub stderr: bool,
}

impl LogConfig {
    /// Daily files `oxikube.<date>.log` in `dir`, seven days kept, no stderr, `RUST_LOG` honoured.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            file_prefix: "oxikube".to_owned(),
            max_files: 7,
            directives: None,
            honour_rust_log: true,
            stderr: false,
        }
    }
}

/// Why logging could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    /// The log directory or file could not be created.
    #[error("cannot create the log file in {dir}: {reason}")]
    File {
        /// The directory.
        dir: PathBuf,
        /// What the OS or the appender said.
        reason: String,
    },
    /// A global `tracing` subscriber (or `log` logger) is already installed.
    #[error("logging is already initialised")]
    AlreadyInitialised,
    /// Filter directives did not parse.
    #[error("invalid log filter `{directives}`: {reason}")]
    Directives {
        /// The text given.
        directives: String,
        /// The parser's message.
        reason: String,
    },
}

/// What [`LogHandle::set_directives`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOutcome {
    /// The new filter is in effect.
    Applied,
    /// `RUST_LOG` chose the filter at start-up and keeps it; nothing changed.
    PinnedByRustLog,
}

/// Changes the log filter of a running subscriber.
#[derive(Clone)]
pub struct LogHandle {
    reload: reload::Handle<EnvFilter, Registry>,
    pinned: bool,
    current: Arc<std::sync::Mutex<String>>,
}

impl std::fmt::Debug for LogHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogHandle")
            .field("directives", &self.directives())
            .field("pinned_by_rust_log", &self.pinned)
            .finish()
    }
}

impl LogHandle {
    /// Replaces the filter with `directives` (`RUST_LOG` syntax). An invalid text is an error and
    /// leaves the filter as it was.
    pub fn set_directives(&self, directives: &str) -> Result<SetOutcome, LogError> {
        if self.pinned {
            return Ok(SetOutcome::PinnedByRustLog);
        }
        let filter = EnvFilter::try_new(directives).map_err(|err| LogError::Directives {
            directives: directives.to_owned(),
            reason: err.to_string(),
        })?;
        self.reload
            .reload(filter)
            .map_err(|err| LogError::Directives {
                directives: directives.to_owned(),
                reason: err.to_string(),
            })?;
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = directives.to_owned();
        Ok(SetOutcome::Applied)
    }

    /// The directives in effect (the text last applied or the start-up filter's).
    pub fn directives(&self) -> String {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether `RUST_LOG` pinned the filter at start-up.
    pub fn is_pinned_by_rust_log(&self) -> bool {
        self.pinned
    }
}

/// Keeps logging alive. Dropping it flushes the queued lines and stops the writer thread, so
/// hold it for the life of the process (`main` does) and drop it last.
#[must_use = "dropping the guard stops file logging"]
pub struct LogGuard {
    _worker: WorkerGuard,
    handle: LogHandle,
    dir: PathBuf,
}

impl LogGuard {
    /// The filter handle (cheap to clone).
    pub fn handle(&self) -> LogHandle {
        self.handle.clone()
    }

    /// The directory the log files are in.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }
}

impl std::fmt::Debug for LogGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LogGuard")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// Builds the subscriber of [`init`] without installing it.
///
/// Use `tracing::subscriber::with_default` (or `set_default`) to run code under it. Dropping the
/// returned guard flushes the file.
pub fn build(config: &LogConfig) -> Result<(Dispatch, LogGuard), LogError> {
    // Created here so the appender's prune of old files finds the directory (it prints to stderr
    // otherwise on a first run).
    std::fs::create_dir_all(&config.dir).map_err(|err| LogError::File {
        dir: config.dir.clone(),
        reason: err.to_string(),
    })?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(&config.file_prefix)
        .filename_suffix("log")
        .max_log_files(config.max_files.max(1))
        .build(&config.dir)
        .map_err(|err| LogError::File {
            dir: config.dir.clone(),
            reason: err.to_string(),
        })?;
    let (writer, worker) = NonBlockingBuilder::default()
        .buffered_lines_limit(BUFFERED_LINES)
        .lossy(true)
        .thread_name("oxikube-log")
        .finish(appender);

    let (filter, source) = initial_filter(config);
    let (filter, reload) = reload::Layer::new(filter);
    let subscriber = Registry::default()
        .with(filter)
        .with(redacting_layer(writer))
        .with(config.stderr.then(|| redacting_layer(std::io::stderr)));

    let handle = LogHandle {
        reload,
        pinned: source.pinned,
        current: Arc::new(std::sync::Mutex::new(source.directives)),
    };
    let guard = LogGuard {
        _worker: worker,
        handle,
        dir: config.dir.clone(),
    };
    Ok((Dispatch::new(subscriber), guard))
}

/// Installs the global subscriber described by `config` and bridges the `log` crate into it.
///
/// Fails with [`LogError::AlreadyInitialised`] when a global subscriber exists (the second call of
/// a process, or a test harness that installed one); nothing is changed then.
pub fn init(config: &LogConfig) -> Result<LogGuard, LogError> {
    let (dispatch, guard) = build(config)?;
    dispatch
        .try_init()
        .map_err(|_| LogError::AlreadyInitialised)?;
    Ok(guard)
}

struct FilterSource {
    directives: String,
    pinned: bool,
}

/// `RUST_LOG` (when honoured and valid), else the configured directives (when valid), else the
/// shipped default.
fn initial_filter(config: &LogConfig) -> (EnvFilter, FilterSource) {
    if config.honour_rust_log
        && let Ok(filter) = EnvFilter::try_from_default_env()
    {
        let directives = std::env::var("RUST_LOG").unwrap_or_default();
        return (
            filter,
            FilterSource {
                directives,
                pinned: true,
            },
        );
    }
    if let Some(text) = &config.directives
        && let Ok(filter) = EnvFilter::try_new(text)
    {
        return (
            filter,
            FilterSource {
                directives: text.clone(),
                pinned: false,
            },
        );
    }
    (
        EnvFilter::new(DEFAULT_DIRECTIVES),
        FilterSource {
            directives: DEFAULT_DIRECTIVES.to_owned(),
            pinned: false,
        },
    )
}
