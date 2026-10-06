//! `init` installs the process-wide subscriber and the `log` bridge, once. One test: the
//! subscriber is global.

use oxikube_logging::{LogConfig, LogError, init};

// Fake secret. Nothing here is a real credential.
const BEARER: &str = "s3cr3t-bearer-token-0123456789";

#[test]
fn init_installs_the_subscriber_and_the_log_bridge_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = LogConfig::new(dir.path());
    config.honour_rust_log = false;
    let guard = init(&config).expect("first init");

    tracing::info!("from tracing");
    // GPUI and most dependencies use the `log` facade: it must land in the same file.
    log::info!("from log: Authorization: Bearer {BEARER}");

    let again = init(&config);
    assert!(
        matches!(again, Err(LogError::AlreadyInitialised)),
        "{again:?}"
    );

    drop(guard);
    let text: String = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert!(text.contains("from tracing"), "{text}");
    assert!(text.contains("from log"), "{text}");
    assert!(!text.contains(BEARER), "{text}");
}
