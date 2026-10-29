//! `ingressclass::SetDefault` end to end on a real cluster (E12-S14): the app on its real init
//! order with the kind context connected, the command sent through its bus and guard, and the
//! annotation read back from the API server. Two throwaway IngressClasses (cluster-scoped, named
//! `oxi-test-<rand>-a` / `-b`) are created and deleted; nothing else in the cluster is touched.
//!
//! The command clears the default flag of every other class, which on a shared cluster would
//! reach a class this test does not own: when the cluster already has a default IngressClass of
//! its own, the test says so and stops before sending anything.
//!
//! `cargo test -p oxikube --features integration --test kind_network` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the test returns at once.
#![cfg(feature = "integration")]

#[path = "kind_common/mod.rs"]
mod kind_common;

use futures::executor::block_on;
use gpui::TestAppContext;
use kube::api::{
    Api, ApiResource, DeleteParams, DynamicObject, GroupVersionKind, ListParams, PostParams,
};
use oxikube::app_state::AppState;
use oxikube_app::command_bus::{CommandBus, DispatchContext, Outcome};
use oxikube_app::{Confirmation, DispatchError};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_ports::AuditQuery;
use oxikube_testkit::integration::{ensure_kind_context, random_namespace_name, test_context};
use serde_json::json;

use kind_common::{launch, on_cluster, wait_value};

const DEFAULT: &str = "ingressclass.kubernetes.io/is-default-class";

fn classes(client: kube::Client) -> Api<DynamicObject> {
    let resource = ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk("networking.k8s.io", "v1", "IngressClass"),
        "ingressclasses",
    );
    Api::all_with(client, &resource)
}

/// The classes the test made, deleted when it ends (also when it fails).
struct Made {
    context: String,
    names: Vec<String>,
}

impl Drop for Made {
    fn drop(&mut self) {
        let names = std::mem::take(&mut self.names);
        on_cluster(&self.context, async |client| {
            let api = classes(client);
            for name in names {
                api.delete(&name, &DeleteParams::default()).await.ok();
            }
        });
    }
}

fn create(context: &str, name: &str, default: bool) {
    let annotations = if default {
        json!({ DEFAULT: "true" })
    } else {
        json!({})
    };
    let manifest: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "networking.k8s.io/v1", "kind": "IngressClass",
        "metadata": {"name": name, "annotations": annotations},
        "spec": {"controller": "oxikube.dev/test"}
    }))
    .expect("a class");
    on_cluster(context, async |client| {
        classes(client)
            .create(&PostParams::default(), &manifest)
            .await
            .expect("create the class");
    });
}

/// The default annotation of the class `name`, from the cluster.
fn flag(context: &str, name: &str) -> Option<String> {
    on_cluster(context, async |client| {
        let class = classes(client).get(name).await.expect("the class");
        class.metadata.annotations?.get(DEFAULT).cloned()
    })
}

/// The names of the classes that are the default now.
fn defaults(context: &str) -> Vec<String> {
    on_cluster(context, async |client| {
        let list = classes(client)
            .list(&ListParams::default())
            .await
            .expect("list");
        list.items
            .into_iter()
            .filter(|c| {
                c.metadata
                    .annotations
                    .as_ref()
                    .and_then(|a| a.get(DEFAULT))
                    .map(String::as_str)
                    == Some("true")
            })
            .filter_map(|c| c.metadata.name)
            .collect()
    })
}

/// Sends `command` on the app's Kubernetes runtime, where its handler's requests need to run
/// (the test's own thread has no reactor), and waits for the outcome, running the app meanwhile.
fn dispatch(
    app: &mut kind_common::Launched,
    bus: &CommandBus,
    command: Command,
    context: DispatchContext,
) -> Result<Outcome, DispatchError> {
    let bus = bus.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let task = app.vcx.update(|_, cx| {
        oxikube_runtime::spawn_kube(cx, async move {
            let _ = tx.send(bus.dispatch(command, context).await);
        })
    });
    let outcome = wait_value(&mut app.vcx, "the command to finish", |_| {
        rx.try_recv().ok()
    });
    drop(task);
    outcome
}

fn target(cluster: &ClusterId, name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(
        cluster.clone(),
        Gvk::new("networking.k8s.io", "v1", "IngressClass"),
        name,
    )
}

#[gpui::test]
fn set_default_moves_the_flag_on_a_real_cluster_through_the_guard(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    let existing = defaults(&context);
    if !existing.is_empty() {
        // Not ours to change; say why nothing was checked.
        tracing::warn!(
            ?existing,
            "skipping: the cluster has default IngressClasses of its own"
        );
        return;
    }
    let stem = random_namespace_name();
    let (a, b) = (format!("{stem}-a"), format!("{stem}-b"));
    let made = Made {
        context: context.clone(),
        names: vec![a.clone(), b.clone()],
    };
    create(&context, &a, true);
    create(&context, &b, false);

    cx.executor().allow_parking();
    let mut app = launch(cx, &context);
    let state = app.vcx.read(AppState::global);
    let bus = state.command_bus().expect("the bus").clone();
    let cluster = app.cluster.clone();
    let command = |name: &str| Command::IngressClassSetDefault {
        target: target(&cluster, name),
        clear_previous: true,
    };
    let ui = || DispatchContext::new(Initiator::Ui, "kind-network-test");

    // The guard asks first, and nothing changed.
    let outcome = dispatch(&mut app, &bus, command(&b), ui()).expect("dispatches");
    let Outcome::NeedsConfirmation(request) = outcome else {
        panic!("expected a confirmation request");
    };
    assert_eq!(
        flag(&context, &b),
        None,
        "nothing is sent before the answer"
    );

    // Confirmed: b is the default, a is not.
    let done = dispatch(
        &mut app,
        &bus,
        command(&b),
        ui().with_confirmation(Confirmation::simple(request.token)),
    )
    .expect("runs");
    assert!(matches!(done, Outcome::Completed(_)));
    app.vcx.run_until_parked();
    assert_eq!(flag(&context, &b).as_deref(), Some("true"));
    assert_eq!(
        flag(&context, &a),
        None,
        "the previous default lost the flag"
    );
    assert_eq!(defaults(&context), [b.as_str()]);

    // A dry run changes nothing, even confirmed.
    let ask = dispatch(&mut app, &bus, command(&a), ui().with_dry_run(true)).expect("dispatches");
    let Outcome::NeedsConfirmation(request) = ask else {
        panic!("expected a confirmation request");
    };
    dispatch(
        &mut app,
        &bus,
        command(&a),
        ui().with_dry_run(true)
            .with_confirmation(Confirmation::simple(request.token)),
    )
    .expect("the dry run runs");
    assert_eq!(defaults(&context), [b.as_str()], "a dry run writes nothing");

    // Read-only: refused for every initiator, nothing written.
    state
        .services()
        .sessions
        .set_read_only(&app.cluster, true)
        .expect("a session");
    for initiator in [Initiator::Ui, Initiator::Command, Initiator::Agent] {
        let refused = dispatch(
            &mut app,
            &bus,
            command(&a),
            DispatchContext::new(initiator, "x"),
        );
        assert!(
            matches!(refused, Err(DispatchError::ReadOnly { .. })),
            "{initiator}: {refused:?}"
        );
    }
    assert_eq!(defaults(&context), [b.as_str()]);

    // The audit log has the attempts by name, with their outcomes.
    let query = AuditQuery {
        cluster: Some(app.cluster.clone()),
        cmd: Some("ingressclass::SetDefault".to_owned()),
        ..AuditQuery::default()
    };
    let records = wait_value(&mut app.vcx, "the audit records", |_| {
        let records = block_on(state.state().query_audit(&query))
            .expect("the audit log")
            .records;
        (records.len() >= 5).then_some(records)
    });
    let outcomes: Vec<_> = records.iter().map(|r| (r.dry_run, r.outcome)).collect();
    assert!(
        outcomes.contains(&(false, AuditOutcome::Succeeded)),
        "{outcomes:?}"
    );
    assert!(
        outcomes.contains(&(true, AuditOutcome::Succeeded)),
        "{outcomes:?}"
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|(_, outcome)| *outcome == AuditOutcome::Denied)
            .count(),
        3,
        "{outcomes:?}"
    );
    drop(made);
}
