//! Kind integration: a restricted service account gets `Forbidden`, not a crash, and the
//! capabilities probe reports its limited rights (E03-S09; error classification from
//! E03-S04, capabilities from E03-S05). Needs `cargo xtask kind-up` and
//! `OXIKUBE_TEST_CONTEXT`; skips cleanly otherwise.
#![cfg(feature = "integration")]

mod common;

use std::sync::Arc;

use k8s_openapi::api::core::v1::{Pod, Secret};
use k8s_openapi::api::rbac::v1::PolicyRule;
use kube::api::ListParams;
use kube::{Api, Client};
use oxikube_domain::ids::{ContextName, Gvr};
use oxikube_domain::{Capability, ErrorKind};
use oxikube_kube::ClientPool;
use oxikube_kube::auth::{CredentialRefresh, classify};
use oxikube_kube::health::{
    AccessLevel, AccessQuery, RBAC_DERIVED, RulesCache, can_i, capabilities_for_context,
    fetch_rules,
};
use oxikube_testkit::integration::TestNamespace;

use common::{DEADLINE, Kind, TestServiceAccount, wait_until, whoami};

/// A service account allowed only `get`/`list` on pods in its own namespace, its
/// context in a pool, and the client the pool built for it.
struct Restricted {
    ns: TestNamespace,
    account: TestServiceAccount,
    context: ContextName,
    pool: ClientPool,
    client: Arc<Client>,
}

impl Restricted {
    /// Creates the account and waits until RBAC lets it list pods, so later checks
    /// see the Role in effect rather than a not-yet-applied binding.
    async fn create(kind: &Kind) -> Self {
        let ns = TestNamespace::create(kind.context.as_str()).expect("test namespace");
        let admin = kind.admin_client().await;
        let pods_only = PolicyRule {
            api_groups: Some(vec![String::new()]),
            resources: Some(vec!["pods".into()]),
            verbs: vec!["get".into(), "list".into()],
            ..PolicyRule::default()
        };
        let account =
            TestServiceAccount::create(&admin, ns.name(), "oxi-restricted", vec![pods_only]).await;

        let context = ContextName::from("oxi-restricted");
        let pool = kind.pool(kind.with_token_context(context.as_str(), &account.token));
        let client = pool.get(&context).await.expect("restricted client");

        let pods = Api::<Pod>::namespaced((*client).clone(), ns.name());
        wait_until("the Role to allow listing pods", DEADLINE, || async {
            pods.list(&ListParams::default()).await.ok()
        })
        .await;
        Self {
            ns,
            account,
            context,
            pool,
            client,
        }
    }

    fn namespace(&self) -> &str {
        self.ns.name()
    }
}

#[tokio::test]
async fn restricted_account_gets_forbidden() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let restricted = Restricted::create(&kind).await;
    let client = &restricted.client;
    assert_eq!(
        whoami(client).await.expect("whoami"),
        restricted.account.username
    );

    // Not allowed: secrets in the same namespace, and pods anywhere else.
    let denied = [
        Api::<Secret>::namespaced((**client).clone(), restricted.namespace())
            .list(&ListParams::default())
            .await
            .map(|_| ()),
        Api::<Pod>::namespaced((**client).clone(), "kube-system")
            .list(&ListParams::default())
            .await
            .map(|_| ()),
    ];
    for result in denied {
        let err = result.expect_err("the account may not do this");
        let classified = classify(&err);
        assert_eq!(classified.kind(), ErrorKind::Forbidden, "{classified:?}");
        assert!(!classified.is_retryable());
        // The apiserver names the identity and the resource; never the credential.
        assert!(
            classified.message().contains("oxi-restricted"),
            "{classified}"
        );
        assert!(!format!("{classified:?}").contains(&restricted.account.token));
    }
}

#[tokio::test]
async fn restricted_account_reports_limited_capabilities() {
    let Some(kind) = common::kind().await else {
        return;
    };
    let restricted = Restricted::create(&kind).await;
    let ns = restricted.namespace();
    let refresh = CredentialRefresh::Static;

    // The rules review lists the Role's rule: pods, get and list, nothing else of ours.
    let rules = fetch_rules(&restricted.client, ns, refresh)
        .await
        .expect("rules review");
    let pod_rules: Vec<_> = rules
        .rules
        .iter()
        .filter(|r| r.resources.iter().any(|res| res == "pods"))
        .collect();
    assert_eq!(pod_rules.len(), 1, "{rules:?}");
    let mut verbs = pod_rules[0].verbs.clone();
    verbs.sort();
    assert_eq!(verbs, ["get", "list"]);
    assert!(
        !rules
            .rules
            .iter()
            .any(|r| r.resources.iter().any(|res| res == "secrets" || res == "*")),
        "{rules:?}"
    );

    // Reduced to session capabilities: no RBAC-derived capability is available.
    let report = capabilities_for_context(
        &restricted.pool,
        &RulesCache::default(),
        &restricted.context,
        ns,
        refresh,
    )
    .await
    .expect("capabilities through the pool");
    assert!(report.available().is_empty(), "{report:?}");
    // kind authorizes with RBAC, so the review is complete and "not granted" is "denied".
    assert!(!rules.is_partial(), "{rules:?}");
    assert_eq!(report.denied(), RBAC_DERIVED, "{report:?}");
    for capability in [Capability::Mutate, Capability::Exec] {
        assert_eq!(
            report.level(capability),
            AccessLevel::Denied,
            "{capability:?}"
        );
    }

    // Per-action checks agree: pods get/list only; nothing on secrets, no exec.
    let pods = Gvr::new("", "v1", "pods");
    let secrets = Gvr::new("", "v1", "secrets");
    let checks = [
        (AccessQuery::new("get", pods.clone()), true),
        (AccessQuery::new("list", pods.clone()), true),
        (AccessQuery::new("delete", pods.clone()), false),
        (
            AccessQuery::new("create", pods.clone()).subresource("exec"),
            false,
        ),
        (AccessQuery::new("get", secrets.clone()), false),
        (AccessQuery::new("list", secrets.clone()), false),
        (AccessQuery::new("create", secrets.clone()), false),
        (AccessQuery::new("delete", secrets), false),
    ];
    for (query, allowed) in checks {
        let query = query.namespace(ns);
        let decision = can_i(&restricted.client, &query, refresh)
            .await
            .expect("access review");
        assert_eq!(decision.is_allowed(), allowed, "{query:?}: {decision:?}");
    }

    // The cluster admin, in the same namespace, has every RBAC-derived capability.
    let admin = capabilities_for_context(
        &kind.pool(kind.kubeconfig.clone()),
        &RulesCache::default(),
        &kind.context,
        ns,
        refresh,
    )
    .await
    .expect("admin capabilities");
    assert_eq!(admin.granted, RBAC_DERIVED, "{admin:?}");
    assert!(admin.denied().is_empty());
}
