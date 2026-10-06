//! The kind cluster under test and the fixtures the smoke tests create in it.
//!
//! The kubeconfig is loaded with the adapter's own loader and cut down to the kind context
//! alone: other contexts in a developer's kubeconfig (cloud clusters with exec plugins) are
//! never built or contacted. The extra contexts are added in memory and never written to disk;
//! their tokens live in this process only.

use std::collections::BTreeMap;
use std::path::PathBuf;

use k8s_openapi::api::authentication::v1::{TokenRequest, TokenRequestSpec};
use k8s_openapi::api::core::v1::{Container, Pod, PodSpec, ServiceAccount};
use k8s_openapi::api::rbac::v1::{PolicyRule, Role, RoleBinding, RoleRef, Subject};
use kube::api::{ObjectMeta, PostParams};
use kube::config::{AuthInfo, Context, Kubeconfig, NamedAuthInfo, NamedContext};
use kube::{Api, Client};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_kube::kubeconfig::{Strictness, default_kubeconfig_path, load_local_kubeconfig};
use oxikube_kube::{ClientPool, ContextDefinition, PoolConfig};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::images;
use oxikube_testkit::integration::{ensure_kind_context, test_context};

/// The scheduler no cluster runs: a pod naming it stays `Pending` and costs the real
/// scheduler nothing.
const NO_SCHEDULER: &str = "oxikube-test-no-scheduler";

/// The label every pod of one test carries, so a cluster-wide feed sees this test's pods and
/// not those of tests running beside it.
pub const RUN_LABEL_KEY: &str = "oxikube.test/smoke";

/// The kind cluster from `OXIKUBE_TEST_CONTEXT`.
pub struct Kind {
    /// The kind kubectl context, e.g. `kind-oxikube`.
    pub context: ContextName,
    /// A kubeconfig holding only the kind context, its cluster and its admin user.
    pub kubeconfig: Kubeconfig,
}

impl Kind {
    /// The cluster to test against, or `None` (the test then returns early) when
    /// `OXIKUBE_TEST_CONTEXT` is unset.
    pub async fn from_env() -> Option<Self> {
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
        Some(Self {
            context,
            kubeconfig: definition.kubeconfig().clone(),
        })
    }

    /// The kind admin's client.
    pub async fn admin_client(&self) -> Client {
        let pool = ClientPool::new(self.kubeconfig.clone(), PoolConfig::default());
        (*pool.get(&self.context).await.expect("admin client")).clone()
    }

    /// Adds a context `name` on the kind cluster whose user authenticates with `token`.
    pub fn add_token_context(&self, kubeconfig: &mut Kubeconfig, name: &str, token: &str) {
        let cluster = self.kubeconfig.contexts[0]
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
    }
}

/// The catalog entry of `context` as a source would list it.
pub fn catalog_entry(context: &ContextName) -> ClusterContext {
    ClusterContext {
        cluster: ClusterId::new("kind-smoke", context),
        context: context.clone(),
        source: SourceId("kind-smoke".into()),
        server: None,
        default_namespace: None,
    }
}

/// A service account in `namespace` that may `get`, `list` and `watch` pods there and nothing
/// else, and a short-lived token for it (TokenRequest API). The token is a secret: it is
/// handed to the in-memory kubeconfig and never printed.
pub async fn read_only_pod_viewer(admin: &Client, namespace: &str, name: &str) -> String {
    let meta = || ObjectMeta {
        name: Some(name.to_owned()),
        namespace: Some(namespace.to_owned()),
        ..ObjectMeta::default()
    };
    let pp = PostParams::default();
    Api::<ServiceAccount>::namespaced(admin.clone(), namespace)
        .create(
            &pp,
            &ServiceAccount {
                metadata: meta(),
                ..ServiceAccount::default()
            },
        )
        .await
        .expect("create service account");
    let role = Role {
        metadata: meta(),
        rules: Some(vec![PolicyRule {
            api_groups: Some(vec![String::new()]),
            resources: Some(vec!["pods".into()]),
            verbs: ["get", "list", "watch"].map(String::from).to_vec(),
            ..PolicyRule::default()
        }]),
    };
    Api::<Role>::namespaced(admin.clone(), namespace)
        .create(&pp, &role)
        .await
        .expect("create role");
    let binding = RoleBinding {
        metadata: meta(),
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
    let request = TokenRequest {
        spec: Some(TokenRequestSpec {
            expiration_seconds: Some(600),
            ..TokenRequestSpec::default()
        }),
        ..TokenRequest::default()
    };
    Api::<ServiceAccount>::namespaced(admin.clone(), namespace)
        .create_token_request(name, &pp, &request)
        .await
        .expect("mint token")
        .status
        .and_then(|status| status.token)
        .expect("minted token")
}

/// Creates `count` pods `<prefix>-<i>` in `namespace`, labelled `RUN_LABEL_KEY=<run>`. They name
/// a scheduler nobody runs, so they stay `Pending` and cost the cluster nothing.
pub async fn create_pods(client: &Client, namespace: &str, run: &str, prefix: &str, count: usize) {
    let api = Api::<Pod>::namespaced(client.clone(), namespace);
    for i in 0..count {
        let pod = Pod {
            metadata: ObjectMeta {
                name: Some(format!("{prefix}-{i}")),
                labels: Some(BTreeMap::from([(RUN_LABEL_KEY.to_owned(), run.to_owned())])),
                ..ObjectMeta::default()
            },
            spec: Some(PodSpec {
                scheduler_name: Some(NO_SCHEDULER.to_owned()),
                termination_grace_period_seconds: Some(1),
                containers: vec![Container {
                    name: "pause".into(),
                    image: Some(images::PAUSE.into()),
                    ..Container::default()
                }],
                ..PodSpec::default()
            }),
            ..Pod::default()
        };
        api.create(&PostParams::default(), &pod)
            .await
            .expect("create pod");
    }
}
