//! The pieces: the spec and its command, the masking, the progress table.

use std::time::Duration;

use oxikube_domain::helm::{ChartRef, ReleaseRef};

use super::{line, spec};
use crate::helm_install::handler::mask_secrets;
use crate::helm_install::{InstallSpec, ReleaseProgress};
use crate::testing::id;

#[test]
fn a_spec_round_trips_through_its_command() {
    let mut original = spec();
    original.wait = true;
    original.timeout = Some(Duration::from_secs(120));
    assert_eq!(InstallSpec::of(&original.command()), Some(original.clone()));
    let latest = InstallSpec::new(id("a"), "web", "demo", ChartRef::new("nginx", ""));
    assert_eq!(InstallSpec::of(&latest.command()), Some(latest.clone()));
    let request = latest.request(false).unwrap();
    assert_eq!(
        request.timeout,
        oxikube_domain::helm::request::DEFAULT_TIMEOUT
    );
    assert!(request.values.is_empty());
    assert!(!format!("{original:?}").contains("hunter2"));
}

#[test]
fn only_secret_documents_are_masked() {
    let manifest = "apiVersion: v1\nkind: ConfigMap\ndata:\n  greeting: hello\n---\napiVersion: v1\nkind: Secret\nstringData:\n  token: abc123\n---\nkind: Service\n";
    let masked = mask_secrets(manifest);
    assert!(masked.contains("greeting: hello"), "{masked}");
    assert!(!masked.contains("abc123"), "{masked}");
    assert!(masked.contains("kind: Service"));
    assert_eq!(masked.matches("---").count(), 2);
    assert_eq!(mask_secrets("kind: Service\n"), "kind: Service\n");
}

#[test]
fn one_operation_per_release_and_followers_end_with_it() {
    let progress = ReleaseProgress::new();
    let web = ReleaseRef::new(id("a"), "demo", "web");
    let mut follower = progress.subscribe(&web);
    let publisher = progress.start(&web).unwrap();
    assert!(progress.is_running(&web));
    assert!(
        progress.start(&web).is_err(),
        "a second install of web waits"
    );
    publisher.publish(&line("one"));
    drop(publisher);
    assert_eq!(follower.try_recv().unwrap().text, "one");
    assert!(matches!(
        follower.try_recv(),
        Err(futures::channel::mpsc::TryRecvError::Closed)
    ));
    assert!(!progress.is_running(&web));
    // A follower whose install never started is forgotten.
    let mut lost = progress.subscribe(&web);
    progress.forget(&web);
    assert!(matches!(
        lost.try_recv(),
        Err(futures::channel::mpsc::TryRecvError::Closed)
    ));
}
