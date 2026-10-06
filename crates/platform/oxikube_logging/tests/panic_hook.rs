//! The installed hook, end to end: a real panic writes a redacted report, logs it, and still
//! reaches the hook that was installed before. One test in this file: the hook is process-global.

use oxikube_logging::{CrashConfig, install_panic_hook};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

// Fake secret. Nothing here is a real credential.
const BEARER: &str = "s3cr3t-bearer-token-0123456789";

static PREVIOUS_RAN: AtomicBool = AtomicBool::new(false);

#[test]
fn a_panic_leaves_a_redacted_crash_file_and_calls_the_previous_hook() {
    let dir = tempfile::tempdir().unwrap();
    let crashes = dir.path().join("crashes");

    std::panic::set_hook(Box::new(|_| PREVIOUS_RAN.store(true, Ordering::SeqCst)));
    assert!(install_panic_hook(CrashConfig::new(&crashes, "1.2.3")));
    assert!(
        !install_panic_hook(CrashConfig::new(dir.path().join("other"), "1.2.3")),
        "a second install is refused"
    );

    let caught = catch_unwind(AssertUnwindSafe(|| {
        panic!("upstream said no: Authorization: Bearer {BEARER}");
    }));
    assert!(caught.is_err());

    assert!(
        PREVIOUS_RAN.load(Ordering::SeqCst),
        "previous hook not called"
    );
    assert!(!dir.path().join("other").exists());
    let files: Vec<_> = std::fs::read_dir(&crashes)
        .expect("crash directory")
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let text = std::fs::read_to_string(&files[0]).unwrap();
    assert!(text.contains("upstream said no"), "{text}");
    assert!(text.contains("version: 1.2.3"), "{text}");
    assert!(text.contains("panic_hook.rs"), "location missing:\n{text}");
    assert!(!text.contains(BEARER), "token leaked:\n{text}");
}
