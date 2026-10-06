//! The panic hook: a redacted crash report on disk, then the previous hook.
//!
//! [`install_panic_hook`] chains in front of whatever hook is installed (the default one that
//! prints to stderr, or a test harness's). When a thread panics it
//!
//! 1. writes `crash-<time>-<pid>.log` into [`CrashConfig::dir`] (message, location, thread,
//!    version, OS and a backtrace), every byte of it passed through
//!    [`redact`](oxikube_domain::redact::redact) so a token that ended up in a panic message
//!    (an `unwrap` on an error that printed a request, a `{:?}` of a credential struct) is
//!    masked;
//! 2. logs one `error` event with the same redacted message;
//! 3. calls the previous hook, so the usual stderr message and abort/unwind behaviour stay.
//!
//! Nothing is uploaded: the report is a local file the user can attach to an issue (the opt-in
//! crash reporter adapter, `oxikube_crash`, is separate). The hook never panics itself: a failure
//! to write is reported on stderr and a panic inside the hook is skipped, not recursed into.
//! Reports are created with mode `0600` on Unix, and only the newest
//! [`CrashConfig::max_reports`] are kept.

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use oxikube_domain::redact::redact;

/// File name prefix of the reports.
const REPORT_PREFIX: &str = "crash-";
/// File name suffix of the reports.
const REPORT_SUFFIX: &str = ".log";

static INSTALLED: AtomicBool = AtomicBool::new(false);
static IN_HOOK: AtomicBool = AtomicBool::new(false);

/// Where reports go and what they say about the build.
#[derive(Debug, Clone)]
pub struct CrashConfig {
    /// Directory of the reports (created at the first crash).
    pub dir: PathBuf,
    /// The app version, printed in the report.
    pub app_version: String,
    /// Reports kept; older ones are deleted after a new one is written.
    pub max_reports: usize,
}

impl CrashConfig {
    /// Reports in `dir`, twenty kept.
    pub fn new(dir: impl Into<PathBuf>, app_version: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            app_version: app_version.into(),
            max_reports: 20,
        }
    }
}

/// What a panic knew about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanicReport {
    /// The panic message (payload text).
    pub message: String,
    /// `file:line:column` of the `panic!`.
    pub location: Option<String>,
    /// Name of the panicking thread.
    pub thread: Option<String>,
    /// Rendered backtrace (empty when not captured).
    pub backtrace: String,
}

impl PanicReport {
    /// Reads the hook's argument and captures a backtrace.
    pub fn from_hook(info: &PanicHookInfo<'_>) -> Self {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(non-string panic payload)".to_owned());
        Self {
            message,
            location: info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())),
            thread: std::thread::current().name().map(str::to_owned),
            backtrace: std::backtrace::Backtrace::force_capture().to_string(),
        }
    }
}

/// The text of the report file: redacted as a whole.
pub fn render_report(config: &CrashConfig, report: &PanicReport) -> String {
    let text = format!(
        "Oxikube crash report\n\
         version: {version}\n\
         time: {time}\n\
         os: {os} {arch}\n\
         thread: {thread}\n\
         location: {location}\n\
         message: {message}\n\
         \n\
         backtrace:\n{backtrace}\n",
        version = config.app_version,
        time = jiff::Timestamp::now(),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        thread = report.thread.as_deref().unwrap_or("<unnamed>"),
        location = report.location.as_deref().unwrap_or("<unknown>"),
        message = report.message,
        backtrace = report.backtrace,
    );
    redact(&text).into_owned()
}

/// Writes the report for `report` into `config.dir` and prunes old ones. Returns the file.
pub fn write_report(config: &CrashConfig, report: &PanicReport) -> io::Result<PathBuf> {
    fs::create_dir_all(&config.dir)?;
    let text = render_report(config, report);
    let stamp = jiff::Timestamp::now().strftime("%Y%m%dT%H%M%SZ");
    let pid = std::process::id();
    let mut attempt = 0u32;
    let (path, mut file) = loop {
        let name = if attempt == 0 {
            format!("{REPORT_PREFIX}{stamp}-{pid}{REPORT_SUFFIX}")
        } else {
            format!("{REPORT_PREFIX}{stamp}-{pid}-{attempt}{REPORT_SUFFIX}")
        };
        let path = config.dir.join(name);
        match open_new(&path) {
            Ok(file) => break (path, file),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists && attempt < 100 => {
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    };
    file.write_all(text.as_bytes())?;
    file.flush()?;
    prune(&config.dir, config.max_reports, &path);
    Ok(path)
}

fn open_new(path: &Path) -> io::Result<fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

/// Deletes the oldest reports beyond `keep` (names sort by time); never `newest`.
fn prune(dir: &Path, keep: usize, newest: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut reports: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(REPORT_PREFIX) && n.ends_with(REPORT_SUFFIX))
        })
        .collect();
    reports.sort();
    let excess = reports.len().saturating_sub(keep.max(1));
    for old in reports.into_iter().take(excess) {
        if old != newest {
            let _ = fs::remove_file(old);
        }
    }
}

/// Installs the hook described in the module docs above. Returns `false` (and changes nothing)
/// when this process already installed one: install once, early in `main`.
pub fn install_panic_hook(config: CrashConfig) -> bool {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return false;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A panic while reporting a panic must not recurse.
        if !IN_HOOK.swap(true, Ordering::SeqCst) {
            report(&config, info);
            IN_HOOK.store(false, Ordering::SeqCst);
        }
        previous(info);
    }));
    true
}

fn report(config: &CrashConfig, info: &PanicHookInfo<'_>) {
    let panic = PanicReport::from_hook(info);
    match write_report(config, &panic) {
        Ok(path) => eprintln!("oxikube: crash report written to {}", path.display()),
        Err(err) => eprintln!(
            "oxikube: could not write a crash report in {}: {err}",
            config.dir.display()
        ),
    }
    tracing::error!(
        target: "panic",
        location = panic.location.as_deref().unwrap_or("<unknown>"),
        thread = panic.thread.as_deref().unwrap_or("<unnamed>"),
        "panic: {}",
        panic.message
    );
}

#[cfg(test)]
mod tests;
