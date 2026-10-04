//! Shared helpers for the kind integration suite (E03-S09).
//!
//! Every test starts with [`kind`], which returns `None` (the test then returns early)
//! when `OXIKUBE_TEST_CONTEXT` is unset, so `cargo test --workspace` stays green without
//! a cluster. The kubeconfig is loaded with the adapter's own loader (E03-S01) and cut
//! down to the kind context alone: other contexts in a developer's kubeconfig (cloud
//! clusters with exec plugins) are never built, contacted or written anywhere.
//!
//! Namespaced fixtures (service accounts, roles, bindings) live in the test's
//! `oxi-test-<rand>` namespace and go away with it.

// Each test file uses a different subset of these helpers.
#![allow(dead_code)]

use std::future::Future;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use k8s_openapi::api::authentication::v1::{SelfSubjectReview, TokenRequest, TokenRequestSpec};
use k8s_openapi::api::core::v1::ServiceAccount;
use k8s_openapi::api::rbac::v1::{PolicyRule, Role, RoleBinding, RoleRef, Subject};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::{DeleteParams, ObjectMeta, PostParams};
use kube::config::{AuthInfo, Context, Kubeconfig, NamedAuthInfo, NamedContext};
use kube::{Api, Client};
use oxikube_domain::ids::{ContextName, Gvk};
use oxikube_kube::kubeconfig::{Strictness, default_kubeconfig_path, load_local_kubeconfig};
use oxikube_kube::{
    ClientPool, ContextDefinition, KubeClientFactory, PoolConfig, ProxyEnv, SystemClock,
};
use oxikube_testkit::integration::{ensure_kind_context, test_context};

/// Default upper bound for the cluster to reflect a change (RBAC, CRDs, discovery).
pub const DEADLINE: Duration = Duration::from_secs(30);

/// Interval between polls in [`wait_until`].
pub const POLL: Duration = Duration::from_millis(100);

/// The kind cluster under test.
pub struct Kind {
    /// The kind kubectl context, e.g. `kind-oxikube`.
    pub context: ContextName,
    /// A kubeconfig holding only the kind context, its cluster and its admin user.
    pub kubeconfig: Kubeconfig,
}

/// The kind cluster from `OXIKUBE_TEST_CONTEXT`, or `None` to skip the test.
pub async fn kind() -> Option<Kind> {
    let context = test_context()?;
    ensure_kind_context(&context).expect("kind context");
    let home = std::env::var_os("HOME").map(|h| default_kubeconfig_path(&PathBuf::from(h)));
    let loaded = load_local_kubeconfig(
        std::env::var_os("KUBECONFIG"),
        home,
        Strictness::RequireUsable,
    )
    .await
    .expect("load the local kubeconfig");
    let context = ContextName::from(context);
    let definition = ContextDefinition::from_kubeconfig(&loaded.merged, &context)
        .unwrap_or_else(|| panic!("context `{context}` is not in the kubeconfig"));
    Some(Kind {
        context,
        kubeconfig: definition.kubeconfig().clone(),
    })
}

impl Kind {
    /// A pool over `kubeconfig` with the real kube factory and no proxy fallback.
    pub fn pool(&self, kubeconfig: Kubeconfig) -> ClientPool {
        ClientPool::with_parts(
            kubeconfig,
            PoolConfig::default(),
            Arc::new(KubeClientFactory::new(ProxyEnv::default())),
            Arc::new(SystemClock),
        )
    }

    /// The kind admin's client, built through the pool.
    pub async fn admin_client(&self) -> Arc<Client> {
        self.pool(self.kubeconfig.clone())
            .get(&self.context)
            .await
            .expect("admin client")
    }

    /// The kind kubeconfig plus a context `name` on the same cluster whose user
    /// authenticates with `token`. The kind context stays the current one.
    pub fn with_token_context(&self, name: &str, token: &str) -> Kubeconfig {
        let mut kubeconfig = self.kubeconfig.clone();
        let cluster = kubeconfig.contexts[0]
            .context
            .as_ref()
            .expect("kind context body")
            .cluster
            .clone();
        let user = format!("{name}-user");
        kubeconfig.auth_infos.push(NamedAuthInfo {
            name: user.clone(),
            auth_info: Some(AuthInfo {
                token: Some(token.to_owned().into()),
                ..AuthInfo::default()
            }),
            ..NamedAuthInfo::default()
        });
        kubeconfig.contexts.push(NamedContext {
            name: name.to_owned(),
            context: Some(Context {
                cluster,
                user: Some(user),
                ..Context::default()
            }),
            ..NamedContext::default()
        });
        kubeconfig
    }
}

/// A service account in a test namespace, optionally bound to a Role with `rules`,
/// and a short-lived token for it (TokenRequest API).
pub struct TestServiceAccount {
    /// `system:serviceaccount:<namespace>:<name>`.
    pub username: String,
    /// Bearer token. Secret: never print it.
    pub token: String,
}

impl TestServiceAccount {
    /// Creates the account (and a Role + RoleBinding when `rules` is not empty) in
    /// `namespace`, then mints a 10-minute token.
    pub async fn create(
        admin: &Client,
        namespace: &str,
        name: &str,
        rules: Vec<PolicyRule>,
    ) -> Self {
        let meta = |name: &str| ObjectMeta {
            name: Some(name.to_owned()),
            namespace: Some(namespace.to_owned()),
            ..ObjectMeta::default()
        };
        let pp = PostParams::default();
        Api::<ServiceAccount>::namespaced(admin.clone(), namespace)
            .create(
                &pp,
                &ServiceAccount {
                    metadata: meta(name),
                    ..ServiceAccount::default()
                },
            )
            .await
            .expect("create service account");
        if !rules.is_empty() {
            Api::<Role>::namespaced(admin.clone(), namespace)
                .create(
                    &pp,
                    &Role {
                        metadata: meta(name),
                        rules: Some(rules),
                    },
                )
                .await
                .expect("create role");
            let binding = RoleBinding {
                metadata: meta(name),
                role_ref: RoleRef {
                    api_group: Some("rbac.authorization.k8s.io".into()),
                    kind: "Role".into(),
                    name: name.to_owned(),
                },
                subjects: Some(vec![Subject {
                    kind: "ServiceAccount".into(),
                    name: name.to_owned(),
                    namespace: Some(namespace.to_owned()),
                    ..Subject::default()
                }]),
            };
            Api::<RoleBinding>::namespaced(admin.clone(), namespace)
                .create(&pp, &binding)
                .await
                .expect("create role binding");
        }
        let request = TokenRequest {
            spec: Some(TokenRequestSpec {
                expiration_seconds: Some(600),
                ..TokenRequestSpec::default()
            }),
            ..TokenRequest::default()
        };
        let minted = Api::<ServiceAccount>::namespaced(admin.clone(), namespace)
            .create_token_request(name, &pp, &request)
            .await
            .expect("mint token");
        let token = minted
            .status
            .and_then(|status| status.token)
            .expect("minted token");
        Self {
            username: format!("system:serviceaccount:{namespace}:{name}"),
            token,
        }
    }
}

/// The username the API server sees for `client` (SelfSubjectReview, allowed for
/// every authenticated user).
pub async fn whoami(client: &Client) -> Result<String, kube::Error> {
    let review = Api::<SelfSubjectReview>::all(client.clone())
        .create(&PostParams::default(), &SelfSubjectReview::default())
        .await?;
    Ok(review
        .status
        .and_then(|s| s.user_info)
        .and_then(|u| u.username)
        .unwrap_or_default())
}

/// Polls `check` every [`POLL`] until it returns `Some`, failing the test after
/// `deadline`. Real time: kind is a real cluster.
pub async fn wait_until<T, F, Fut>(what: &str, deadline: Duration, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    let started = Instant::now();
    loop {
        if let Some(value) = check().await {
            return value;
        }
        assert!(
            started.elapsed() < deadline,
            "{what} not reached within {deadline:?}"
        );
        tokio::time::sleep(POLL).await;
    }
}

/// A CRD created by a test; deleted (best effort, no wait) when dropped, also on panic.
pub struct TestCrd {
    context: String,
    /// `gizmos.<group>`.
    pub name: String,
    /// `<group>/v1 Gizmo`.
    pub gvk: Gvk,
}

impl TestCrd {
    /// Creates `gizmos.rt-<rand>.test.oxikube.dev` (kind `Gizmo`, version `v1`).
    pub async fn create(client: &Client, context: &str) -> Self {
        let suffix = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let group = format!("rt-{suffix}.test.oxikube.dev");
        let name = format!("gizmos.{group}");
        let crd: CustomResourceDefinition = serde_json::from_value(serde_json::json!({
            "apiVersion": "apiextensions.k8s.io/v1",
            "kind": "CustomResourceDefinition",
            "metadata": {"name": name},
            "spec": {
                "group": group,
                "scope": "Namespaced",
                "names": {
                    "plural": "gizmos", "singular": "gizmo", "kind": "Gizmo",
                    "listKind": "GizmoList", "shortNames": ["gz"],
                },
                "versions": [{
                    "name": "v1", "served": true, "storage": true,
                    "schema": {"openAPIV3Schema": {"type": "object", "x-kubernetes-preserve-unknown-fields": true}},
                }],
            },
        }))
        .expect("crd json");
        // Registered before the create so a failed or interrupted create still cleans up.
        let guard = Self {
            context: context.to_owned(),
            name,
            gvk: Gvk::new(group, "v1", "Gizmo"),
        };
        Api::<CustomResourceDefinition>::all(client.clone())
            .create(&PostParams::default(), &crd)
            .await
            .expect("create CRD");
        guard
    }

    /// Deletes the CRD now.
    pub async fn delete(&self, client: &Client) {
        Api::<CustomResourceDefinition>::all(client.clone())
            .delete(&self.name, &DeleteParams::default())
            .await
            .expect("delete CRD");
    }
}

impl Drop for TestCrd {
    fn drop(&mut self) {
        let _ = Command::new("kubectl")
            .args(["--context", &self.context, "delete", "crd", &self.name])
            .args(["--ignore-not-found", "--wait=false"])
            .output();
    }
}
