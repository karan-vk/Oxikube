//! Shared helpers for the no-leak tests (`debug_redaction*.rs`): a global trace-level capture
//! of formatted tracing output, and assertions that no fixture secret appears in a value's
//! `Debug`, an error in any format, or the captured output.

// Each test binary uses a different subset.
#![allow(dead_code)]

use std::io;
use std::sync::{Arc, LazyLock, Mutex, Once};

use oxikube_domain::OxiError;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

/// Collects all formatted tracing output of this test binary.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

static CAPTURE: LazyLock<Capture> = LazyLock::new(Capture::default);

/// Installs a global trace-level subscriber once. Global, not per-thread: client builds run on
/// blocking-pool threads, which do not inherit a thread-local default.
pub fn init_tracing() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::new("trace"))
            .with_ansi(false)
            .with_writer(CAPTURE.clone())
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("one global subscriber");
    });
}

/// Everything captured so far.
pub fn captured() -> String {
    String::from_utf8_lossy(&CAPTURE.0.lock().unwrap()).into_owned()
}

/// Fails if `text` contains any of `secrets`.
#[track_caller]
pub fn assert_no_secrets(what: &str, text: &str, secrets: &[&str]) {
    for secret in secrets {
        assert!(!text.contains(secret), "{what} leaks {secret:?}:\n{text}");
    }
}

/// `{:?}` and `{:#?}` of `value`, for both checks.
#[track_caller]
pub fn assert_debug_clean(what: &str, value: &dyn std::fmt::Debug, secrets: &[&str]) {
    assert_no_secrets(what, &format!("{value:?}"), secrets);
    assert_no_secrets(what, &format!("{value:#?}"), secrets);
}

/// The error text a caller (and a log line) would show, in every format, down the source chain.
#[track_caller]
pub fn assert_error_clean(what: &str, err: &OxiError, secrets: &[&str]) {
    assert_no_secrets(
        what,
        &format!("{err:?} | {err:#?} | {err} | {}", err.message()),
        secrets,
    );
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        assert_no_secrets(what, &format!("{cause:?} | {cause}"), secrets);
        source = cause.source();
    }
}
