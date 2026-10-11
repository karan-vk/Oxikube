//! The handler's pieces: the namespace an object goes to, the managers' times, the dry-run data.

use oxikube_domain::audit::Initiator;
use serde_json::json;

use super::*;
use crate::apply::manifest::namespace_for;
use crate::apply::service::manager_times;
use crate::command_bus::{DispatchContext, Outcome};

#[test]
fn the_namespace_follows_the_kind_the_manifest_and_the_command() {
    let cases = [
        (Some(true), Some("shop"), Some("other"), Some("shop")),
        (Some(true), None, Some("other"), Some("other")),
        (Some(true), None, None, Some("default")),
        (Some(false), Some("shop"), Some("other"), None),
        (None, Some("shop"), None, Some("shop")),
        (None, None, Some("other"), Some("other")),
        (None, None, None, None),
    ];
    for (namespaced, written, default, expected) in cases {
        assert_eq!(
            namespace_for(namespaced, written, default).as_deref(),
            expected,
            "{namespaced:?} {written:?} {default:?}"
        );
    }
}

#[test]
fn the_newest_write_of_each_manager_counts() {
    let times = manager_times(&live_configmap("1").to_value());
    assert_eq!(times["kubectl"], "2026-10-02T11:00:00Z");
    assert_eq!(times["helm"], "2026-09-30T09:00:00Z");
    assert!(manager_times(&json!({"metadata": {}})).is_empty());
}

#[test]
fn a_dry_run_dispatch_returns_the_object_next_to_the_live_one() {
    let t = Applying::new(false);
    t.port().insert(live_configmap("7"));
    let command = t.request(CONFIGMAP).command(None);
    let ctx = DispatchContext::new(Initiator::Agent, "agent")
        .with_cluster(id("a"))
        .with_dry_run(true);
    let Outcome::Completed(output) = t.h.confirm_and_run(command, ctx).expect("dry run") else {
        panic!("completed")
    };
    assert_eq!(
        output.message.as_deref(),
        Some("Dry run: ConfigMap default/web can be applied")
    );
    let data = output.data.unwrap();
    assert_eq!(data["outcome"], "dry_run");
    assert_eq!(data["method"], "server_side");
    assert_eq!(data["object"]["data"]["mode"], "blue");
    assert_eq!(data["live"]["data"]["mode"], "green");
    assert_eq!(data["resource_version"], "7");
    assert_eq!(t.writes(), [("patch", true)]);
}
