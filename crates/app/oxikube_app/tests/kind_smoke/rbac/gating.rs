//! Epic E17 "done when" (1) and (5) against kind: the account's kubeconfig, generated through
//! `serviceaccount::GenerateKubeconfig`, connects a session whose rules reviews hide what the
//! account may not list (the sidebar's sections and `AccessVisibility`, the `KindVisibility` the
//! sidebar's entries ask) and whose namespace list, forbidden, falls back to the namespaces the
//! account may read in.

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_app::access::AccessVisibility;
use oxikube_app::kinds::KindVisibility;
use oxikube_app::session::namespaces::{NamespaceService, NamespaceSource};
use oxikube_app::sidebar::{AccessOutcome, review_access};
use oxikube_domain::access::AccessRequirement;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::session::NamespaceSelection;
use oxikube_testkit::FakeStatePort;
use oxikube_testkit::integration::rbac::AppliedScenario;

use super::{Admin, Restricted};
use crate::DEADLINE;
use crate::clock::TokioClock;
use crate::cluster::Kind;

/// The sidebar's review for `selection`, through the session's own access port.
async fn review(restricted: &Restricted, selection: &NamespaceSelection) -> AccessOutcome {
    let session = restricted
        .manager
        .get(&restricted.entry.cluster)
        .expect("the session");
    let port = session
        .access()
        .expect("a connected session has an access port");
    review_access(port.as_ref(), selection).await
}

fn offers(outcome: &AccessOutcome, group: &str, resource: &str) -> bool {
    outcome.offers(&[AccessRequirement::list(group, resource)])
}

fn listable(
    visibility: &AccessVisibility,
    cluster: &ClusterId,
    (group, kind, plural): (&str, &str, &str),
    namespace: Option<&str>,
) -> Option<bool> {
    visibility.is_listable_resource(
        cluster,
        &Gvk::new(group, "v1", kind),
        Some(plural),
        namespace,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_generated_kubeconfig_hides_what_the_account_may_not_list() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let applied = AppliedScenario::apply(kind.context.as_str()).expect("the RBAC scenario");
    let scenario = applied.scenario().clone();
    let admin = Admin::connect(&kind).await;
    let generated = admin.generate_kubeconfig(&applied, None).await;
    let home = NamespaceSelection::single(&scenario.home);
    let restricted = Restricted::connect(&generated, home.clone()).await;
    let cluster = restricted.entry.cluster.clone();

    // --- the sidebar's sections: Workloads (pods) stays, Config and the cluster kinds go -------
    let started = Instant::now();
    let outcome = loop {
        let outcome = review(&restricted, &home).await;
        if offers(&outcome, "", "pods") {
            break outcome;
        }
        assert!(
            started.elapsed() < DEADLINE,
            "the Role never reached the authorizer: {outcome:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(outcome.warning(), None, "the review itself worked");
    for (group, resource) in [
        ("", "secrets"),
        ("", "configmaps"),
        ("", "services"),
        ("apps", "deployments"),
        ("", "nodes"),
        ("", "namespaces"),
        ("rbac.authorization.k8s.io", "roles"),
    ] {
        assert!(
            !offers(&outcome, group, resource),
            "{group}/{resource} is hidden: {outcome:?}"
        );
    }
    // Pods in `home` only: selecting `other` hides them too.
    let other = NamespaceSelection::single(&scenario.other);
    assert!(!offers(&review(&restricted, &other).await, "", "pods"));

    // --- the entries' hook (`AccessVisibility`, E17-S03) over the same reviews ----------------
    let namespaces = NamespaceService::new(
        restricted.manager.clone(),
        Arc::new(FakeStatePort::new()),
        Arc::new(TokioClock),
    );
    let visibility = AccessVisibility::new(restricted.manager.clone(), namespaces.access().clone());
    let both = NamespaceSelection::from_names([&scenario.home, &scenario.other]);
    visibility.refresh(&cluster, &both).await;
    let pods = ("", "Pod", "pods");
    assert_eq!(
        listable(&visibility, &cluster, pods, Some(&scenario.home)),
        Some(true)
    );
    assert_eq!(
        listable(&visibility, &cluster, pods, Some(&scenario.other)),
        Some(false)
    );
    assert_eq!(
        listable(
            &visibility,
            &cluster,
            ("", "Secret", "secrets"),
            Some(&scenario.home)
        ),
        Some(false)
    );
    assert_eq!(
        listable(&visibility, &cluster, ("", "Node", "nodes"), None),
        Some(false)
    );

    // --- (5) the namespace list is forbidden: the selector offers where the account reads ----
    restricted
        .manager
        .set_namespace_selection(&cluster, both)
        .expect("select both namespaces");
    let catalog = namespaces.catalog(&cluster).await.expect("a catalog");
    assert_eq!(catalog.source, NamespaceSource::Forbidden, "{catalog:?}");
    assert_eq!(
        catalog.names,
        std::slice::from_ref(&scenario.home),
        "{catalog:?}"
    );
}
