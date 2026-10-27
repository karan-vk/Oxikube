//! Epic E17 "done when" (2) and (3) against kind: the app's subject index, fed by the store's four
//! RBAC subscriptions (`PolicyFeed`, as the subject pages run it), lists for the account exactly
//! what `kubectl auth can-i --list --as=<account>` prints, as rules and as the per-verb matrix;
//! then `binding::Create` (through the guard, risk medium, confirmed) grants it the aggregated
//! ClusterRole `view` in `other`, the account's own access review allows what `view` allows there,
//! and the index and `kubectl` agree again.
//!
//! The comparison is exact: the index holds the cluster's own default bindings (the `system:*`
//! roles every authenticated user gets), so nothing is filtered out. A mismatch saves both sides
//! (`oxikube_testkit::integration::rbac::save_diagnostic`) for the CI artefacts.

use std::sync::Arc;
use std::time::Instant;

use oxikube_app::access::{AccessRegistry, PolicyFeed, PolicyStatus};
use oxikube_app::command_bus::Outcome;
use oxikube_app::store::{ResourceStore, Spawner, StoreOptions, StorePorts, StoreRuntime};
use oxikube_domain::access::{AccessCheck, AccessDecision};
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvr;
use oxikube_domain::rbac::{GrantScope, NormalisedRules, PolicyMatrix, Subject};
use oxikube_domain::session::NamespaceSelection;
use oxikube_kube::{KubeDiscovery, KubeResources};
use oxikube_testkit::fixtures::rbac::printed_rules;
use oxikube_testkit::integration::rbac::{AppliedScenario, can_i_list, save_diagnostic};

use super::{Admin, Restricted};
use crate::clock::TokioClock;
use crate::cluster::Kind;
use crate::{DEADLINE, eventually};

/// The admin's `ResourceStore` (the one a cluster tab's tables read) and the policy feed over it.
async fn policy_feed(kind: &Kind, admin: &Admin) -> (ResourceStore, PolicyFeed) {
    let client = kind.admin_client().await;
    let reader = Arc::new(KubeResources::new(
        client.clone(),
        KubeDiscovery::new(client),
    ));
    let spawner: Arc<dyn Spawner> = Arc::new(|task| {
        tokio::spawn(task);
    });
    let store = ResourceStore::new(
        admin.entry.cluster.clone(),
        StorePorts {
            resources: reader.clone(),
            tables: reader,
        },
        StoreRuntime {
            spawner,
            clock: Arc::new(TokioClock),
            probe: None,
        },
        StoreOptions::default(),
    );
    let feed = PolicyFeed::start(&store);
    eventually(
        "the four RBAC kinds to list",
        || format!("{:?}", feed.status()),
        || async { feed.status() == PolicyStatus::Ready },
    )
    .await;
    (store, feed)
}

/// Waits until the index and `kubectl auth can-i --list` agree on `subject` in `namespace` (the
/// feed and the authorizer both catch up with a change asynchronously), then checks the matrix.
/// On a timeout both sides are saved and the assertion shows the difference.
async fn agree(kind: &Kind, feed: &PolicyFeed, subject: &Subject, namespace: &str, step: &str) {
    let started = Instant::now();
    loop {
        let printed = can_i_list(kind.context.as_str(), subject, namespace).expect("kubectl");
        let theirs = NormalisedRules::from_policy(&printed_rules(&printed));
        let ours = feed.read(|index| index.rules_in(subject, Some(namespace)));
        if ours == theirs {
            let scope = GrantScope::Namespace(namespace.into());
            assert_eq!(
                feed.read(|index| index.matrix(subject, &scope)),
                PolicyMatrix::from_rules(&theirs),
                "{step}: the matrix of {subject} in {namespace}"
            );
            return;
        }
        if started.elapsed() > DEADLINE {
            save_diagnostic(
                &format!("matrix-{step}-{namespace}.txt"),
                &format!(
                    "index:\n{ours:#?}\nkubectl auth can-i --list:\n{}",
                    printed.join("\n")
                ),
            );
            assert_eq!(ours, theirs, "{step}: {subject} in {namespace}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_index_matches_kubectl_before_and_after_a_view_binding() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let applied = AppliedScenario::apply(kind.context.as_str()).expect("the RBAC scenario");
    let scenario = applied.scenario().clone();
    let admin = Admin::connect(&kind).await;
    let (_store, feed) = policy_feed(&kind, &admin).await;
    let account = scenario.account();

    // --- (2) before: pods at home, the defaults elsewhere --------------------------------------
    agree(&kind, &feed, &account, &scenario.home, "before").await;
    agree(&kind, &feed, &account, &scenario.other, "before").await;

    // --- (3) the binding, through the bus and the guard ----------------------------------------
    let outcome = admin
        .run_confirmed(Command::BindingCreate {
            cluster: admin.entry.cluster.clone(),
            manifest: scenario.view_binding().to_string(),
        })
        .await;
    assert!(matches!(outcome, Outcome::Completed(_)), "{outcome:?}");

    // The account's own review (its generated kubeconfig, its access service) now allows what
    // the aggregated `view` allows in `other`, and still not Secrets.
    let generated = admin.generate_kubeconfig(&applied, None).await;
    let restricted =
        Restricted::connect(&generated, NamespaceSelection::single(&scenario.other)).await;
    let session = restricted
        .manager
        .get(&restricted.entry.cluster)
        .expect("the session");
    let access = AccessRegistry::new(Arc::new(TokioClock))
        .for_session(&restricted.entry.cluster, &session)
        .expect("a connected session has an access service");
    let check = |resource: &str| {
        AccessCheck::new("list", Gvr::new("", "v1", resource)).namespace(&scenario.other)
    };
    eventually(
        "the binding to let the account list config maps in `other`",
        || format!("binding {}", scenario.view_binding()),
        || {
            access.invalidate_namespace(Some(&scenario.other));
            let (access, check) = (access.clone(), check("configmaps"));
            async move {
                access
                    .can_i(&check)
                    .await
                    .expect("access review")
                    .is_allowed()
            }
        },
    )
    .await;
    let secrets = access
        .can_i(&check("secrets"))
        .await
        .expect("access review");
    assert!(
        matches!(secrets, AccessDecision::Denied { .. }),
        "`view` leaves Secrets out: {secrets:?}"
    );

    // --- (2) after: the index follows the new binding and still agrees with kubectl -----------
    agree(&kind, &feed, &account, &scenario.other, "after").await;
    agree(&kind, &feed, &account, &scenario.home, "after").await;
}
