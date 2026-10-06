//! What runs before GPUI: logging, the panic hook, the data directory.
//!
//! This is the first stage of start-up because everything after it wants to log, and because a
//! panic anywhere after it must leave a crash file. It reads no settings (they come later, and
//! the `log.filter` setting is applied to the running subscriber when they do) and does no
//! network or heavy work: creating the log file is the only I/O.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use oxikube_logging::{CrashConfig, LogConfig, LogGuard, LogHandle};

use super::paths;
use super::stage::{Stage, StageTiming};

/// Keeps file logging alive until [`shutdown`]; a static because the process may exit from inside
/// GPUI's `run` (macOS), where `main` never regains control to drop it.
static LOG_GUARD: Mutex<Option<LogGuard>> = Mutex::new(None);

/// The result of [`boot`].
pub struct Boot {
    /// The app's data directory, when the OS reports one.
    pub data_dir: Option<PathBuf>,
    /// The log filter handle, when file logging started.
    pub log: Option<LogHandle>,
    /// What the stages that ran here cost.
    pub timings: Vec<StageTiming>,
}

/// Sets up logging and the panic hook. Never fails: a problem is printed to stderr and start-up
/// goes on without that piece (the app must open even when the disk is read-only).
pub fn boot() -> Boot {
    let started = Instant::now();
    let data_dir = paths::data_dir();
    // Logs and crash reports are redacted, so the temp directory is an acceptable last resort.
    let files_dir = data_dir
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("oxikube"));

    let mut config = LogConfig::new(paths::log_dir(&files_dir));
    config.stderr = cfg!(debug_assertions);
    let log = match oxikube_logging::init(&config) {
        Ok(guard) => {
            let handle = guard.handle();
            *LOG_GUARD.lock().unwrap_or_else(|e| e.into_inner()) = Some(guard);
            Some(handle)
        }
        Err(err) => {
            eprintln!("oxikube: logging is off: {err}");
            None
        }
    };
    oxikube_logging::install_panic_hook(CrashConfig::new(
        paths::crash_dir(&files_dir),
        env!("CARGO_PKG_VERSION"),
    ));
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        data_dir = %files_dir.display(),
        "oxikube starting"
    );
    let elapsed = started.elapsed();
    tracing::info!(
        stage = Stage::Logging.name(),
        elapsed_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
        "init stage done"
    );
    Boot {
        data_dir,
        log,
        timings: vec![StageTiming {
            stage: Stage::Logging,
            elapsed,
        }],
    }
}

/// Flushes and stops file logging. Idempotent; call it when the app quits and after `run`
/// returns.
pub fn shutdown() {
    let guard = LOG_GUARD.lock().unwrap_or_else(|e| e.into_inner()).take();
    drop(guard);
}
