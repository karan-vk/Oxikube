//! A mutating action the cached access review denies is greyed in the table, with the verb in
//! its reason (E17-S03); the server and the guard still decide, so an unknown stays enabled.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_domain::access::{AccessRule, AccessRules};
use oxikube_domain::command::CommandId;

use super::{dialog, toasts};
use crate::suite::pod_in;
use crate::table::tests::fixture::{Fixture, cluster};

/// A viewer: reads pods, deletes nothing.
fn viewer() -> AccessRules {
    AccessRules::none().with_rule(AccessRule::granting(
        &["get", "list", "watch"],
        &[""],
        &["pods"],
        &[],
    ))
}

/// Reviews `namespace` as the app does on connect, so the cache holds the answer.
fn review(f: &Fixture, namespace: &str) {
    let session = f.sessions.get(&cluster()).expect("connected");
    let service = f
        .access
        .for_session(&cluster(), &session)
        .expect("a connected session has a service");
    block_on(service.rules(Some(namespace))).expect("rules");
}

#[gpui::test]
fn delete_is_greyed_for_a_viewer_but_enabled_while_the_review_is_unknown(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with([pod_in("x", "web-0", 0)]);
    let table = f.open_pods();
    f.keys(&table, "j");

    // Before any review the action is enabled: unknown never greys.
    let before = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    assert!(
        before
            .iter()
            .find(|e| e.command() == CommandId::POD_DELETE)
            .is_some_and(|e| e.is_enabled()),
        "{before:?}"
    );

    // The viewer's review lands.
    f.ports().access.set_namespace_rules("x", viewer());
    review(&f, "x");
    let entries = f.vcx.update(|_, cx| table.read(cx).action_entries(cx));
    let delete = entries
        .iter()
        .find(|e| e.command() == CommandId::POD_DELETE)
        .expect("Delete is still listed");
    assert!(!delete.is_enabled());
    let reason = delete.reason().expect("a reason");
    assert!(reason.contains("no delete permission"), "{reason}");

    // The key says why instead of opening the dialog; nothing reaches the guard.
    f.keys(&table, "delete");
    assert!(dialog(&mut f).is_none());
    assert!(
        toasts(&mut f)
            .iter()
            .any(|t| t.message.contains("no delete permission")),
        "the user is told why"
    );
    assert!(f.state.audit_log().is_empty());
}
