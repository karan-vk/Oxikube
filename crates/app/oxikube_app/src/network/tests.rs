//! The Service detail's queries, `service::ShowPods` as a plan, and the `service::OpenInBrowser`
//! handler that exists only when an opener is registered.

use std::sync::Arc;

use async_trait::async_trait;
use futures::FutureExt as _;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::session::WatchScope;
use oxikube_domain::view::{NoSelector, ServiceSummary};
use oxikube_domain::{Initiator, OxiError, Resource};
use oxikube_testkit::resource;
use parking_lot::Mutex;
use serde_json::{Value, json};

use super::*;
use crate::command_bus::{DispatchContext, Outcome};
use crate::testing::{Harness, id};

fn service(spec: Value) -> ServiceSummary {
    let res: Resource = resource("v1", "Service")
        .name("web")
        .namespace("demo")
        .field("spec", spec)
        .build();
    ServiceSummary::from_resource(&res).unwrap()
}

fn selecting() -> ServiceSummary {
    service(json!({"selector": {"tier": "front", "app": "web"}, "ports": [{"port": 80}]}))
}

#[test]
fn the_detail_queries_slices_by_service_name_and_pods_by_selector() {
    let backends = ServiceBackends::of(&selecting());
    let scope = WatchScope::Namespaces(vec!["demo".to_owned()]);

    assert_eq!(backends.slices.gvk, ServiceBackends::slice_gvk());
    assert_eq!(backends.slices.scope, scope);
    assert_eq!(
        backends.slices.selector.as_ref().map(ToString::to_string),
        Some("kubernetes.io/service-name=web".to_owned()),
        "the slices are asked for by label, server side"
    );

    let PodsPlan::Query(pods) = backends.pods else {
        panic!("a Service with a selector has a pod query");
    };
    assert_eq!(pods.gvk, Gvk::new("", "v1", "Pod"));
    assert_eq!(
        pods.scope, scope,
        "never wider than the Service's namespace"
    );
    assert_eq!(
        pods.selector.as_ref().map(ToString::to_string),
        Some("app=web,tier=front".to_owned()),
        "the selector is the Service's, sorted"
    );
}

#[test]
fn a_service_without_a_selector_has_no_pod_query_and_says_why() {
    let manual = ServiceBackends::of(&service(json!({"ports": [{"port": 80}]})));
    assert_eq!(manual.pods, PodsPlan::None(NoSelector::Manual));
    assert!(
        manual.slices.selector.is_some(),
        "its slices are still listed: manual endpoints are slices too"
    );

    let external = ServiceBackends::of(&service(
        json!({"type": "ExternalName", "externalName": "db.example.com"}),
    ));
    assert_eq!(external.pods, PodsPlan::None(NoSelector::ExternalName));
}

#[test]
fn show_pods_opens_the_pod_table_filtered_by_the_selector() {
    let cluster = id("a");
    let only_demo = WatchScope::Namespaces(vec!["demo".to_owned()]);
    assert_eq!(
        plan_show_pods(&cluster, &selecting(), &only_demo).unwrap(),
        vec![
            Command::ResourceOpenList {
                cluster: cluster.clone(),
                gvk: Gvk::new("", "v1", "Pod"),
            },
            Command::TableSetFilter {
                cluster: cluster.clone(),
                gvk: Gvk::new("", "v1", "Pod"),
                text: "-l app=web,tier=front".to_owned(),
            },
        ],
        "the selection is already exactly the Service's namespace: it is not touched"
    );
}

#[test]
fn show_pods_narrows_every_other_selection_to_the_services_namespace() {
    let cluster = id("a");
    let select = Command::NamespaceSelect {
        cluster: cluster.clone(),
        namespaces: vec!["demo".to_owned()],
    };
    for (what, selection) in [
        ("all namespaces", WatchScope::Cluster),
        (
            "several, including the Service's",
            WatchScope::Namespaces(vec!["demo".to_owned(), "other".to_owned()]),
        ),
        (
            "another one",
            WatchScope::Namespaces(vec!["kube-system".to_owned()]),
        ),
        ("none", WatchScope::Namespaces(Vec::new())),
    ] {
        let plan = plan_show_pods(&cluster, &selecting(), &selection).unwrap();
        assert_eq!(
            plan[0], select,
            "{what}: a label selector carries no namespace, so pods of other namespaces \
             sharing the labels would be listed"
        );
        assert_eq!(plan.len(), 3, "{what}");
    }
}

#[test]
fn show_pods_refuses_a_service_that_selects_nothing() {
    let cluster = id("a");
    let manual = service(json!({"ports": [{"port": 80}]}));
    assert_eq!(
        plan_show_pods(&cluster, &manual, &WatchScope::Cluster),
        Err(NoSelector::Manual)
    );
    let external = service(json!({"type": "ExternalName", "externalName": "x.example.com"}));
    assert_eq!(
        plan_show_pods(&cluster, &external, &WatchScope::Cluster),
        Err(NoSelector::ExternalName)
    );
}

/// Records the forwards it was asked for.
#[derive(Default)]
struct Recording {
    asked: Mutex<Vec<(ResourceRef, Option<u16>)>>,
    fail: bool,
}

#[async_trait]
impl ServicePortOpener for Recording {
    async fn open_service_port(
        &self,
        service: ResourceRef,
        port: Option<u16>,
    ) -> Result<OpenedServicePort, OxiError> {
        self.asked.lock().push((service, port));
        if self.fail {
            return Err(OxiError::not_found("no ready pod backs the service"));
        }
        Ok(OpenedServicePort {
            url: "http://localhost:41234".to_owned(),
            service_port: port.unwrap_or(80),
        })
    }
}

fn open(port: Option<u16>) -> Command {
    Command::ServiceOpenInBrowser {
        target: ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "Service"), "demo", "web"),
        port,
    }
}

#[test]
fn open_in_browser_asks_the_opener_and_hands_the_url_to_the_ui() {
    let opener = Arc::new(Recording::default());
    let urls = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = urls.clone();
    let h = Harness::with_extra(|reg, _, _| {
        register_open_in_browser(
            reg,
            opener.clone(),
            Arc::new(move |url| sink.lock().push(url)),
        )
    });
    h.connect("a", false);

    let ctx = DispatchContext::new(Initiator::Ui, "alice").with_cluster(id("a"));
    let outcome = h
        .bus
        .dispatch(open(Some(8080)), ctx)
        .now_or_never()
        .expect("the fake answers at once")
        .expect("dispatched");
    let Outcome::Completed(output) = outcome else {
        panic!("a forward needs no confirmation: {outcome:?}");
    };
    assert_eq!(*urls.lock(), ["http://localhost:41234"]);
    assert_eq!(
        output.message.as_deref(),
        Some("Opened http://localhost:41234")
    );
    assert_eq!(
        opener.asked.lock().as_slice(),
        [(open_target(), Some(8080))]
    );
}

fn open_target() -> ResourceRef {
    ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "Service"), "demo", "web")
}

#[test]
fn a_failed_forward_opens_nothing_and_reports_why() {
    let opener = Arc::new(Recording {
        fail: true,
        ..Recording::default()
    });
    let urls = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = urls.clone();
    let h = Harness::with_extra(|reg, _, _| {
        register_open_in_browser(reg, opener.clone(), Arc::new(move |u| sink.lock().push(u)))
    });
    h.connect("a", false);
    let ctx = DispatchContext::new(Initiator::Ui, "alice").with_cluster(id("a"));
    let result = h
        .bus
        .dispatch(open(None), ctx)
        .now_or_never()
        .expect("answers at once");
    assert!(result.is_err(), "the opener's error reaches the caller");
    assert!(urls.lock().is_empty());
}

#[test]
fn without_an_opener_the_command_is_not_on_the_bus() {
    let h = Harness::with_extra(|_, _, _| Ok(()));
    assert!(
        !h.bus.is_registered(CommandId::SERVICE_OPEN_IN_BROWSER),
        "nothing registered it: the row action and the palette never offer it"
    );
}

#[test]
fn neither_command_is_a_mutation_so_a_read_only_cluster_still_runs_them() {
    use oxikube_domain::Capabilities;
    use oxikube_domain::command::lookup;

    use crate::actions::ActionContext;

    let read_only = ActionContext::new(Capabilities::all()).read_only(true);
    for id in [
        CommandId::SERVICE_SHOW_PODS,
        CommandId::SERVICE_OPEN_IN_BROWSER,
    ] {
        let meta = lookup(id).expect("declared");
        assert!(!meta.mutating, "{id} changes nothing in the cluster");
        assert!(
            read_only.state_of(meta).is_enabled(),
            "{id} stays enabled on a read-only cluster"
        );
    }
    // A forward needs the session to be able to forward at all.
    let without = ActionContext::new(Capabilities::empty());
    let meta = lookup(CommandId::SERVICE_OPEN_IN_BROWSER).unwrap();
    assert!(meta.needs.contains(Capabilities::PORTFORWARD));
    assert!(!without.capabilities.contains(meta.needs));
}
