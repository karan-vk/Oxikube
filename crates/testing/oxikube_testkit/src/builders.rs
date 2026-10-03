//! Short builders for domain [`Resource`]s: `pod().running().restarts(3).build()`.
//!
//! Builders produce the same shape as the JSON fixtures (`crate::fixtures`), with the same
//! defaults, so a view-model test can mix the two:
//!
//! | Default | Value |
//! |---|---|
//! | namespace | [`NAMESPACE`] (`demo`) |
//! | `metadata.creationTimestamp` | [`CREATED`] |
//! | container | `app`, image [`IMAGE`], requests `100m` / `128Mi` (QoS `Burstable`) |
//! | node | [`NODE`] once the pod is scheduled |
//! | pod IP | [`POD_IP`] once the pod has started |
//!
//! Every builder ends with `build()` (or `Resource::from(builder)`); `json()` returns the
//! raw object instead.

use std::collections::BTreeMap;

use oxikube_domain::Resource;
use serde_json::{Map, Value, json};

/// Default namespace of namespaced builders and fixtures.
pub const NAMESPACE: &str = "demo";
/// Default `metadata.creationTimestamp`.
pub const CREATED: &str = "2026-01-01T00:00:00Z";
/// When a container started (`CREATED` + 5 s).
pub const STARTED: &str = "2026-01-01T00:00:05Z";
/// When the last restart finished (`CREATED` + 1 h); set when restarts > 0.
pub const LAST_RESTART: &str = "2026-01-01T01:00:00Z";
/// When a terminating object was deleted (`CREATED` + 2 h).
pub const DELETED: &str = "2026-01-01T02:00:00Z";
/// Default container image.
pub const IMAGE: &str = "registry.example/app:1.0";
/// Default node of scheduled pods (and name of [`node()`]).
pub const NODE: &str = "worker-1";
/// Default pod IP of started pods.
pub const POD_IP: &str = "10.244.1.10";
/// `hostIP` of pods on [`NODE`] and `InternalIP` of [`node()`].
pub const NODE_IP: &str = "172.18.0.3";

/// Metadata shared by every builder.
#[derive(Debug, Clone)]
struct Meta {
    name: String,
    namespace: Option<String>,
    uid: Option<String>,
    labels: BTreeMap<String, String>,
    annotations: BTreeMap<String, String>,
    created: String,
    deleted: Option<String>,
}

impl Meta {
    fn new(name: &str, namespaced: bool) -> Self {
        Self {
            name: name.to_owned(),
            namespace: namespaced.then(|| NAMESPACE.to_owned()),
            uid: None,
            labels: BTreeMap::new(),
            annotations: BTreeMap::new(),
            created: CREATED.to_owned(),
            deleted: None,
        }
    }

    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("name".into(), json!(self.name));
        if let Some(ns) = &self.namespace {
            m.insert("namespace".into(), json!(ns));
        }
        if let Some(uid) = &self.uid {
            m.insert("uid".into(), json!(uid));
        }
        m.insert("creationTimestamp".into(), json!(self.created));
        if let Some(d) = &self.deleted {
            m.insert("deletionTimestamp".into(), json!(d));
            m.insert("deletionGracePeriodSeconds".into(), json!(30));
        }
        if !self.labels.is_empty() {
            m.insert("labels".into(), json!(self.labels));
        }
        if !self.annotations.is_empty() {
            m.insert("annotations".into(), json!(self.annotations));
        }
        Value::Object(m)
    }
}

/// Adds the metadata setters (`name`, `namespace`, `uid`, `label`, `annotation`,
/// `created`) and `build`/`From` to a builder with a `meta: Meta` field and a
/// `fn json(&self) -> Value`.
macro_rules! meta_builder {
    ($builder:ty) => {
        impl $builder {
            /// Sets `metadata.name`.
            #[must_use]
            pub fn name(mut self, name: impl Into<String>) -> Self {
                self.meta.name = name.into();
                self
            }

            /// Sets `metadata.namespace`.
            #[must_use]
            pub fn namespace(mut self, namespace: impl Into<String>) -> Self {
                self.meta.namespace = Some(namespace.into());
                self
            }

            /// Sets `metadata.uid`.
            #[must_use]
            pub fn uid(mut self, uid: impl Into<String>) -> Self {
                self.meta.uid = Some(uid.into());
                self
            }

            /// Adds a label.
            #[must_use]
            pub fn label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
                self.meta.labels.insert(key.into(), value.into());
                self
            }

            /// Adds an annotation.
            #[must_use]
            pub fn annotation(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
                self.meta.annotations.insert(key.into(), value.into());
                self
            }

            /// Sets `metadata.creationTimestamp` (RFC 3339).
            #[must_use]
            pub fn created(mut self, at: impl Into<String>) -> Self {
                self.meta.created = at.into();
                self
            }

            /// Builds the domain [`Resource`].
            ///
            /// # Panics
            ///
            /// Only if the builder produced JSON that is not a valid `Resource` (for
            /// example an empty name or a malformed timestamp set by the test).
            pub fn build(&self) -> Resource {
                match Resource::from_json(self.json()) {
                    Ok(res) => res,
                    Err(e) => panic!("{} built an invalid Resource: {e}", stringify!($builder)),
                }
            }
        }

        impl From<$builder> for Resource {
            fn from(builder: $builder) -> Self {
                builder.build()
            }
        }
    };
}

// --- Pod ---------------------------------------------------------------------------------

/// The state a [`PodBuilder`] renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PodState {
    Pending,
    ContainerCreating,
    Running,
    Succeeded,
    Failed,
    CrashLoop,
    ImagePullBackOff,
    OomKilled,
    Init { done: u32, total: u32 },
    NodeLost,
}

/// Builder for a one-container Pod. Start with [`pod()`]; the default state is
/// [`running`](Self::running).
#[derive(Debug, Clone)]
pub struct PodBuilder {
    meta: Meta,
    state: PodState,
    restarts: Option<u32>,
    image: String,
    node: Option<String>,
    ip: Option<String>,
    terminating: bool,
}

/// A running, ready pod named `pod` in [`NAMESPACE`].
pub fn pod() -> PodBuilder {
    PodBuilder {
        meta: Meta::new("pod", true),
        state: PodState::Running,
        restarts: None,
        image: IMAGE.to_owned(),
        node: None,
        ip: None,
        terminating: false,
    }
}

meta_builder!(PodBuilder);

impl PodBuilder {
    /// Unschedulable: phase `Pending`, no node, no container status. Status `Pending`.
    #[must_use]
    pub fn pending(mut self) -> Self {
        self.state = PodState::Pending;
        self
    }

    /// Scheduled, container waiting `ContainerCreating`.
    #[must_use]
    pub fn container_creating(mut self) -> Self {
        self.state = PodState::ContainerCreating;
        self
    }

    /// Running and ready. Status `Running`.
    #[must_use]
    pub fn running(mut self) -> Self {
        self.state = PodState::Running;
        self
    }

    /// Phase `Succeeded`, container terminated `Completed`. Status `Completed`.
    #[must_use]
    pub fn succeeded(mut self) -> Self {
        self.state = PodState::Succeeded;
        self
    }

    /// Phase `Failed`, container terminated `Error` (exit 1). Status `Error`.
    #[must_use]
    pub fn failed(mut self) -> Self {
        self.state = PodState::Failed;
        self
    }

    /// Container waiting `CrashLoopBackOff` after exiting with `Error`; 5 restarts
    /// unless [`restarts`](Self::restarts) says otherwise.
    #[must_use]
    pub fn crash_loop(mut self) -> Self {
        self.state = PodState::CrashLoop;
        self
    }

    /// Container waiting `ImagePullBackOff`.
    #[must_use]
    pub fn image_pull_backoff(mut self) -> Self {
        self.state = PodState::ImagePullBackOff;
        self
    }

    /// Container terminated `OOMKilled` (exit 137). Status `OOMKilled`.
    #[must_use]
    pub fn oom_killed(mut self) -> Self {
        self.state = PodState::OomKilled;
        self
    }

    /// `done` of `total` init containers finished; the next one is running. Status
    /// `Init:done/total`.
    #[must_use]
    pub fn init(mut self, done: u32, total: u32) -> Self {
        self.state = PodState::Init { done, total };
        self
    }

    /// The node stopped reporting: `status.reason` `NodeLost`, container not ready.
    #[must_use]
    pub fn node_lost(mut self) -> Self {
        self.state = PodState::NodeLost;
        self
    }

    /// Running but deleted (`deletionTimestamp` = [`DELETED`]). Status `Terminating`.
    #[must_use]
    pub fn terminating(mut self) -> Self {
        self.terminating = true;
        self
    }

    /// Sets the container's `restartCount`; a non-zero count also records the last
    /// termination (finished at [`LAST_RESTART`]).
    #[must_use]
    pub fn restarts(mut self, count: u32) -> Self {
        self.restarts = Some(count);
        self
    }

    /// Sets the container image.
    #[must_use]
    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }

    /// Sets the node (also for a pending pod, which then counts as scheduled).
    #[must_use]
    pub fn node(mut self, node: impl Into<String>) -> Self {
        self.node = Some(node.into());
        self
    }

    /// Sets the pod IP.
    #[must_use]
    pub fn ip(mut self, ip: impl Into<String>) -> Self {
        self.ip = Some(ip.into());
        self
    }

    /// The Pod as JSON.
    pub fn json(&self) -> Value {
        let mut meta = self.meta.clone();
        if self.terminating && meta.deleted.is_none() {
            meta.deleted = Some(DELETED.to_owned());
        }
        let state = self.state;
        let restarts = self
            .restarts
            .unwrap_or(if state == PodState::CrashLoop { 5 } else { 0 });
        let scheduled = state != PodState::Pending || self.node.is_some();
        let started = matches!(
            state,
            PodState::Running
                | PodState::Succeeded
                | PodState::Failed
                | PodState::CrashLoop
                | PodState::OomKilled
                | PodState::NodeLost
                | PodState::Init { .. }
        );

        let container = json!({
            "name": "app",
            "image": self.image,
            "ports": [{"containerPort": 8080, "protocol": "TCP"}],
            "resources": {"requests": {"cpu": "100m", "memory": "128Mi"}}
        });
        let mut spec = json!({
            "containers": [container],
            "restartPolicy": "Always",
            "serviceAccountName": "default"
        });
        if scheduled {
            spec["nodeName"] = json!(self.node.as_deref().unwrap_or(NODE));
        }

        let (phase, ready) = match state {
            PodState::Pending
            | PodState::ContainerCreating
            | PodState::ImagePullBackOff
            | PodState::Init { .. } => ("Pending", false),
            PodState::Running => ("Running", true),
            PodState::Succeeded => ("Succeeded", false),
            PodState::Failed => ("Failed", false),
            PodState::CrashLoop | PodState::OomKilled | PodState::NodeLost => ("Running", false),
        };

        let mut status = json!({"phase": phase, "qosClass": "Burstable"});
        status["conditions"] = if scheduled {
            json!([
                {"type": "Initialized", "status": bool_str(!matches!(state, PodState::Init { .. }))},
                {"type": "Ready", "status": bool_str(ready)},
                {"type": "ContainersReady", "status": bool_str(ready)},
                {"type": "PodScheduled", "status": "True"}
            ])
        } else {
            json!([{
                "type": "PodScheduled", "status": "False", "reason": "Unschedulable",
                "message": "0/1 nodes are available: 1 Insufficient cpu."
            }])
        };
        if scheduled {
            status["hostIP"] = json!(NODE_IP);
            status["startTime"] = json!(STARTED);
        }
        if started && !matches!(state, PodState::Init { .. }) {
            let ip = self.ip.as_deref().unwrap_or(POD_IP);
            status["podIP"] = json!(ip);
            status["podIPs"] = json!([{"ip": ip}]);
        }
        if state == PodState::NodeLost {
            status["reason"] = json!("NodeLost");
            status["message"] = json!(format!("Node {NODE} which was running pod is unresponsive"));
        }

        if let PodState::Init { done, total } = state {
            let inits: Vec<Value> = (0..total)
                .map(|i| json!({"name": format!("init-{i}"), "image": "registry.example/init:1.0"}))
                .collect();
            spec["initContainers"] = json!(inits);
            let init_statuses: Vec<Value> = (0..total)
                .map(|i| {
                    let state = if i < done {
                        json!({"terminated": {"exitCode": 0, "reason": "Completed",
                            "startedAt": STARTED, "finishedAt": STARTED}})
                    } else if i == done {
                        json!({"running": {"startedAt": STARTED}})
                    } else {
                        json!({"waiting": {"reason": "PodInitializing"}})
                    };
                    json!({"name": format!("init-{i}"), "image": "registry.example/init:1.0",
                        "ready": i < done, "started": i == done, "restartCount": 0, "state": state})
                })
                .collect();
            status["initContainerStatuses"] = json!(init_statuses);
        }

        if !matches!(state, PodState::Pending) {
            let cstate = match state {
                PodState::ContainerCreating | PodState::Init { .. } => {
                    let reason = if matches!(state, PodState::Init { .. }) {
                        "PodInitializing"
                    } else {
                        "ContainerCreating"
                    };
                    json!({"waiting": {"reason": reason}})
                }
                PodState::ImagePullBackOff => json!({"waiting": {
                    "reason": "ImagePullBackOff",
                    "message": format!("Back-off pulling image \"{}\"", self.image)
                }}),
                PodState::CrashLoop => json!({"waiting": {
                    "reason": "CrashLoopBackOff",
                    "message": "back-off 5m0s restarting failed container=app"
                }}),
                PodState::Succeeded => json!({"terminated": {
                    "exitCode": 0, "reason": "Completed", "startedAt": STARTED, "finishedAt": LAST_RESTART
                }}),
                PodState::Failed => json!({"terminated": {
                    "exitCode": 1, "reason": "Error", "startedAt": STARTED, "finishedAt": LAST_RESTART
                }}),
                PodState::OomKilled => json!({"terminated": {
                    "exitCode": 137, "reason": "OOMKilled", "startedAt": STARTED, "finishedAt": LAST_RESTART
                }}),
                PodState::Running | PodState::NodeLost | PodState::Pending => {
                    json!({"running": {"startedAt": STARTED}})
                }
            };
            let mut cs = json!({
                "name": "app",
                "image": self.image,
                "ready": ready,
                "started": matches!(state, PodState::Running | PodState::NodeLost),
                "restartCount": restarts,
                "state": cstate
            });
            if restarts > 0 {
                cs["lastState"] = json!({"terminated": {
                    "exitCode": 1, "reason": "Error", "startedAt": STARTED, "finishedAt": LAST_RESTART
                }});
            }
            status["containerStatuses"] = json!([cs]);
        }

        json!({"apiVersion": "v1", "kind": "Pod", "metadata": meta.to_json(), "spec": spec, "status": status})
    }
}

fn bool_str(b: bool) -> &'static str {
    if b { "True" } else { "False" }
}

// --- Workloads ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkloadKind {
    Deployment,
    StatefulSet,
    DaemonSet,
    ReplicaSet,
}

/// Builder for a Deployment, StatefulSet, DaemonSet or ReplicaSet. Start with
/// [`deployment()`], [`statefulset()`], [`daemonset()`] or [`replicaset()`]. By default
/// it is settled: every replica current, updated, ready and available.
#[derive(Debug, Clone)]
pub struct WorkloadBuilder {
    meta: Meta,
    kind: WorkloadKind,
    replicas: u32,
    ready: Option<u32>,
    updated: Option<u32>,
    available: Option<u32>,
    paused: bool,
    partition: Option<u32>,
    image: String,
}

fn workload(kind: WorkloadKind, name: &str) -> WorkloadBuilder {
    WorkloadBuilder {
        meta: Meta::new(name, true).with_label("app", name),
        kind,
        replicas: if kind == WorkloadKind::DaemonSet {
            2
        } else {
            1
        },
        ready: None,
        updated: None,
        available: None,
        paused: false,
        partition: None,
        image: IMAGE.to_owned(),
    }
}

impl Meta {
    fn with_label(mut self, key: &str, value: &str) -> Self {
        self.labels.insert(key.to_owned(), value.to_owned());
        self
    }
}

/// A settled one-replica Deployment named `web` (label `app=web`).
pub fn deployment() -> WorkloadBuilder {
    workload(WorkloadKind::Deployment, "web")
}

/// A settled one-replica StatefulSet named `db` (label `app=db`).
pub fn statefulset() -> WorkloadBuilder {
    workload(WorkloadKind::StatefulSet, "db")
}

/// A settled DaemonSet named `agent` scheduled on 2 nodes (label `app=agent`).
pub fn daemonset() -> WorkloadBuilder {
    workload(WorkloadKind::DaemonSet, "agent")
}

/// A settled one-replica ReplicaSet named `web-5d8c7b9f4` (label `app=web`).
pub fn replicaset() -> WorkloadBuilder {
    let mut b = workload(WorkloadKind::ReplicaSet, "web-5d8c7b9f4");
    b.meta.labels.insert("app".into(), "web".into());
    b
}

meta_builder!(WorkloadBuilder);

impl WorkloadBuilder {
    /// Desired replicas (for a DaemonSet: desired scheduled nodes).
    #[must_use]
    pub fn replicas(mut self, n: u32) -> Self {
        self.replicas = n;
        self
    }

    /// Ready replicas (default: all). Available follows unless set.
    #[must_use]
    pub fn ready(mut self, n: u32) -> Self {
        self.ready = Some(n);
        self
    }

    /// Updated replicas (default: all).
    #[must_use]
    pub fn updated(mut self, n: u32) -> Self {
        self.updated = Some(n);
        self
    }

    /// Available replicas (default: the ready count).
    #[must_use]
    pub fn available(mut self, n: u32) -> Self {
        self.available = Some(n);
        self
    }

    /// Paused rollout (Deployments only).
    #[must_use]
    pub fn paused(mut self) -> Self {
        self.paused = true;
        self
    }

    /// Rolling-update partition (StatefulSets only).
    #[must_use]
    pub fn partition(mut self, n: u32) -> Self {
        self.partition = Some(n);
        self
    }

    /// Container image of the pod template.
    #[must_use]
    pub fn image(mut self, image: impl Into<String>) -> Self {
        self.image = image.into();
        self
    }

    /// The object as JSON.
    pub fn json(&self) -> Value {
        let n = self.replicas;
        let ready = self.ready.unwrap_or(n);
        let updated = self.updated.unwrap_or(n);
        let available = self.available.unwrap_or(ready);
        let app = self
            .meta
            .labels
            .get("app")
            .cloned()
            .unwrap_or_else(|| self.meta.name.clone());
        let template = json!({
            "metadata": {"labels": {"app": app}},
            "spec": {"containers": [{"name": "app", "image": self.image}]}
        });
        let selector = json!({"matchLabels": {"app": app}});
        let (kind, spec, status) = match self.kind {
            WorkloadKind::Deployment => {
                let mut spec = json!({"replicas": n, "selector": selector, "template": template});
                if self.paused {
                    spec["paused"] = json!(true);
                }
                let status = json!({
                    "observedGeneration": 1, "replicas": n, "updatedReplicas": updated,
                    "readyReplicas": ready, "availableReplicas": available,
                    "unavailableReplicas": n.saturating_sub(available)
                });
                ("Deployment", spec, status)
            }
            WorkloadKind::StatefulSet => {
                let mut spec = json!({"replicas": n, "selector": selector, "template": template,
                    "serviceName": self.meta.name});
                if let Some(p) = self.partition {
                    spec["updateStrategy"] =
                        json!({"type": "RollingUpdate", "rollingUpdate": {"partition": p}});
                }
                let status = json!({
                    "observedGeneration": 1, "replicas": n, "currentReplicas": n,
                    "updatedReplicas": updated, "readyReplicas": ready,
                    "availableReplicas": available
                });
                ("StatefulSet", spec, status)
            }
            WorkloadKind::DaemonSet => {
                let spec = json!({"selector": selector, "template": template});
                let status = json!({
                    "observedGeneration": 1, "desiredNumberScheduled": n,
                    "currentNumberScheduled": n, "numberMisscheduled": 0,
                    "updatedNumberScheduled": updated, "numberReady": ready,
                    "numberAvailable": available
                });
                ("DaemonSet", spec, status)
            }
            WorkloadKind::ReplicaSet => {
                let spec = json!({"replicas": n, "selector": selector, "template": template});
                let status = json!({
                    "observedGeneration": 1, "replicas": n, "fullyLabeledReplicas": n,
                    "readyReplicas": ready, "availableReplicas": available
                });
                ("ReplicaSet", spec, status)
            }
        };
        json!({"apiVersion": "apps/v1", "kind": kind, "metadata": self.meta.to_json(),
            "spec": spec, "status": status})
    }
}

// --- Job ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobState {
    Running,
    Complete,
    Failed,
}

/// Builder for a batch/v1 Job. Start with [`job()`]; the default state is running.
#[derive(Debug, Clone)]
pub struct JobBuilder {
    meta: Meta,
    state: JobState,
    completions: u32,
}

/// A running one-completion Job named `migrate`.
pub fn job() -> JobBuilder {
    JobBuilder {
        meta: Meta::new("migrate", true),
        state: JobState::Running,
        completions: 1,
    }
}

meta_builder!(JobBuilder);

impl JobBuilder {
    /// Still running: one active pod, nothing finished.
    #[must_use]
    pub fn running(mut self) -> Self {
        self.state = JobState::Running;
        self
    }

    /// Finished: every completion succeeded (`Complete` condition).
    #[must_use]
    pub fn complete(mut self) -> Self {
        self.state = JobState::Complete;
        self
    }

    /// Gave up: `Failed` condition (`BackoffLimitExceeded`), 6 failed pods.
    #[must_use]
    pub fn failed(mut self) -> Self {
        self.state = JobState::Failed;
        self
    }

    /// `spec.completions` (default 1).
    #[must_use]
    pub fn completions(mut self, n: u32) -> Self {
        self.completions = n;
        self
    }

    /// The Job as JSON.
    pub fn json(&self) -> Value {
        let spec = json!({
            "completions": self.completions, "parallelism": 1, "backoffLimit": 6,
            "template": {"spec": {"restartPolicy": "Never",
                "containers": [{"name": "app", "image": IMAGE}]}}
        });
        let status = match self.state {
            JobState::Running => json!({"active": 1, "startTime": STARTED}),
            JobState::Complete => json!({
                "succeeded": self.completions, "startTime": STARTED,
                "completionTime": LAST_RESTART,
                "conditions": [
                    {"type": "SuccessCriteriaMet", "status": "True"},
                    {"type": "Complete", "status": "True"}
                ]
            }),
            JobState::Failed => json!({
                "failed": 6, "startTime": STARTED,
                "conditions": [
                    {"type": "FailureTarget", "status": "True", "reason": "BackoffLimitExceeded"},
                    {"type": "Failed", "status": "True", "reason": "BackoffLimitExceeded",
                        "message": "Job has reached the specified backoff limit"}
                ]
            }),
        };
        json!({"apiVersion": "batch/v1", "kind": "Job", "metadata": self.meta.to_json(),
            "spec": spec, "status": status})
    }
}

// --- Node --------------------------------------------------------------------------------

/// Builder for a Node. Start with [`node()`]; the default is a ready, schedulable worker.
#[derive(Debug, Clone)]
pub struct NodeBuilder {
    meta: Meta,
    ready: bool,
    cordoned: bool,
    roles: Vec<String>,
    kubelet: String,
    conditions: Vec<(String, bool)>,
}

/// A ready, schedulable node named [`NODE`] with no role.
pub fn node() -> NodeBuilder {
    NodeBuilder {
        meta: Meta::new(NODE, false)
            .with_label("kubernetes.io/hostname", NODE)
            .with_label("kubernetes.io/os", "linux"),
        ready: true,
        cordoned: false,
        roles: Vec::new(),
        kubelet: "v1.33.1".to_owned(),
        conditions: Vec::new(),
    }
}

meta_builder!(NodeBuilder);

impl NodeBuilder {
    /// `Ready` condition `True` (the default).
    #[must_use]
    pub fn ready(mut self) -> Self {
        self.ready = true;
        self
    }

    /// `Ready` condition `False` (kubelet stopped posting). Status `NotReady`.
    #[must_use]
    pub fn not_ready(mut self) -> Self {
        self.ready = false;
        self
    }

    /// `spec.unschedulable` with the matching taint. Status `Ready,SchedulingDisabled`.
    #[must_use]
    pub fn cordoned(mut self) -> Self {
        self.cordoned = true;
        self
    }

    /// Adds a `node-role.kubernetes.io/<role>` label.
    #[must_use]
    pub fn role(mut self, role: impl Into<String>) -> Self {
        self.roles.push(role.into());
        self
    }

    /// Sets `status.nodeInfo.kubeletVersion` (default `v1.33.1`).
    #[must_use]
    pub fn kubelet(mut self, version: impl Into<String>) -> Self {
        self.kubelet = version.into();
        self
    }

    /// Sets a pressure condition (`MemoryPressure`, `DiskPressure`, `PIDPressure`,
    /// `NetworkUnavailable`) to `True` or `False`. Unset ones are `False`.
    #[must_use]
    pub fn condition(mut self, kind: impl Into<String>, status: bool) -> Self {
        self.conditions.push((kind.into(), status));
        self
    }

    /// The Node as JSON.
    pub fn json(&self) -> Value {
        let mut meta = self.meta.clone();
        for role in &self.roles {
            meta.labels
                .insert(format!("node-role.kubernetes.io/{role}"), String::new());
        }
        let mut spec = json!({"podCIDR": "10.244.1.0/24"});
        if self.cordoned {
            spec["unschedulable"] = json!(true);
            spec["taints"] = json!([{"key": "node.kubernetes.io/unschedulable",
                "effect": "NoSchedule", "timeAdded": STARTED}]);
        }
        let mut conditions: Vec<Value> = ["MemoryPressure", "DiskPressure", "PIDPressure"]
            .iter()
            .map(|kind| {
                let status = self
                    .conditions
                    .iter()
                    .rev()
                    .find(|(k, _)| k == kind)
                    .is_some_and(|(_, s)| *s);
                json!({"type": kind, "status": bool_str(status)})
            })
            .collect();
        for (kind, status) in &self.conditions {
            if !["MemoryPressure", "DiskPressure", "PIDPressure"].contains(&kind.as_str()) {
                conditions.push(json!({"type": kind, "status": bool_str(*status)}));
            }
        }
        conditions.push(if self.ready {
            json!({"type": "Ready", "status": "True", "reason": "KubeletReady",
                "message": "kubelet is posting ready status"})
        } else {
            json!({"type": "Ready", "status": "Unknown", "reason": "NodeStatusUnknown",
                "message": "Kubelet stopped posting node status."})
        });
        let status = json!({
            "addresses": [
                {"type": "InternalIP", "address": NODE_IP},
                {"type": "Hostname", "address": self.meta.name}
            ],
            "allocatable": {"cpu": "4", "memory": "16Gi", "pods": "110", "ephemeral-storage": "100Gi"},
            "capacity": {"cpu": "4", "memory": "16Gi", "pods": "110", "ephemeral-storage": "100Gi"},
            "conditions": conditions,
            "nodeInfo": {
                "architecture": "amd64",
                "containerRuntimeVersion": "containerd://2.1.1",
                "kernelVersion": "6.8.0",
                "kubeletVersion": self.kubelet,
                "operatingSystem": "linux",
                "osImage": "Debian GNU/Linux 12 (bookworm)"
            }
        });
        json!({"apiVersion": "v1", "kind": "Node", "metadata": meta.to_json(),
            "spec": spec, "status": status})
    }
}

// --- Anything else -----------------------------------------------------------------------

/// Builder for any kind (CRs, ConfigMaps, ...): metadata plus arbitrary top-level fields.
/// Start with [`resource()`].
#[derive(Debug, Clone)]
pub struct ResourceBuilder {
    meta: Meta,
    api_version: String,
    kind: String,
    fields: Map<String, Value>,
}

/// A namespaced object of `api_version` / `kind` named `example` (call
/// [`cluster_scoped`](ResourceBuilder::cluster_scoped) for cluster-scoped kinds).
pub fn resource(api_version: impl Into<String>, kind: impl Into<String>) -> ResourceBuilder {
    ResourceBuilder {
        meta: Meta::new("example", true),
        api_version: api_version.into(),
        kind: kind.into(),
        fields: Map::new(),
    }
}

meta_builder!(ResourceBuilder);

impl ResourceBuilder {
    /// Drops `metadata.namespace`.
    #[must_use]
    pub fn cluster_scoped(mut self) -> Self {
        self.meta.namespace = None;
        self
    }

    /// Sets a top-level field such as `spec`, `status` or `data`.
    #[must_use]
    pub fn field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.fields.insert(key.into(), value);
        self
    }

    /// The object as JSON.
    pub fn json(&self) -> Value {
        let mut obj = Map::new();
        obj.insert("apiVersion".into(), json!(self.api_version));
        obj.insert("kind".into(), json!(self.kind));
        obj.insert("metadata".into(), self.meta.to_json());
        for (k, v) in &self.fields {
            obj.insert(k.clone(), v.clone());
        }
        Value::Object(obj)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::view::PodPhase;
    use oxikube_domain::{JobSummary, NodeSummary, PodSummary, WorkloadSummary};

    fn status(b: PodBuilder) -> String {
        PodSummary::from_resource(&b.build())
            .unwrap()
            .status
            .to_string()
    }

    #[test]
    fn pod_states_print_the_kubectl_status() {
        let cases = [
            (pod().pending(), "Pending"),
            (pod().container_creating(), "ContainerCreating"),
            (pod().running(), "Running"),
            (pod().succeeded(), "Completed"),
            (pod().failed(), "Error"),
            (pod().crash_loop(), "CrashLoopBackOff"),
            (pod().image_pull_backoff(), "ImagePullBackOff"),
            (pod().oom_killed(), "OOMKilled"),
            (pod().init(1, 2), "Init:1/2"),
            (pod().running().terminating(), "Terminating"),
            (pod().node_lost(), "NodeLost"),
        ];
        for (builder, expected) in cases {
            assert_eq!(status(builder.clone()), expected, "{builder:?}");
        }
    }

    #[test]
    fn pod_builder_sets_restarts_ready_and_metadata() {
        let res = pod()
            .name("web")
            .namespace("prod")
            .label("app", "web")
            .running()
            .restarts(3)
            .build();
        let s = PodSummary::from_resource(&res).unwrap();
        assert_eq!((&*s.name, s.namespace.as_deref()), ("web", Some("prod")));
        assert_eq!(
            (s.phase, s.ready, s.total, s.restarts),
            (PodPhase::Running, 1, 1, 3)
        );
        assert_eq!(s.last_restart, Some(LAST_RESTART.parse().unwrap()));
        assert_eq!(s.node.as_deref(), Some(NODE));
        assert_eq!(s.ip.as_deref(), Some(POD_IP));
        assert_eq!(res.meta.labels.get("app").map(|v| &**v), Some("web"));
        assert_eq!(
            PodSummary::from_resource(&pod().crash_loop().build())
                .unwrap()
                .restarts,
            5
        );
        let pending = PodSummary::from_resource(&pod().pending().build()).unwrap();
        assert_eq!((pending.node, pending.ip), (None, None));
        let r: Resource = pod().into();
        assert_eq!(r.name(), "pod");
    }

    #[test]
    fn workload_builders_report_replicas() {
        let d = WorkloadSummary::from_resource(&deployment().replicas(3).ready(2).build()).unwrap();
        assert_eq!((d.desired, d.ready, d.available, d.updated), (3, 2, 2, 3));
        assert_eq!(d.ready_display(), "2/3");
        assert!(!d.is_settled());
        assert!(
            WorkloadSummary::from_resource(&deployment().paused().build())
                .unwrap()
                .paused
        );
        let s = WorkloadSummary::from_resource(&statefulset().replicas(3).partition(1).build())
            .unwrap();
        assert_eq!((s.desired, s.partition), (3, Some(1)));
        let ds = WorkloadSummary::from_resource(&daemonset().ready(1).build()).unwrap();
        assert_eq!((ds.desired, ds.ready), (2, 1));
        let rs = WorkloadSummary::from_resource(&replicaset().replicas(2).build()).unwrap();
        assert!(rs.is_settled());
    }

    #[test]
    fn node_and_job_builders() {
        let n = NodeSummary::from_resource(&node().build()).unwrap();
        assert_eq!(&*n.status, "Ready");
        assert!(n.schedulable);
        let c =
            NodeSummary::from_resource(&node().cordoned().role("control-plane").build()).unwrap();
        assert_eq!(&*c.status, "Ready,SchedulingDisabled");
        assert_eq!(c.roles_display(), "control-plane");
        let nr = NodeSummary::from_resource(&node().not_ready().build()).unwrap();
        assert_eq!(&*nr.status, "NotReady");
        let p =
            NodeSummary::from_resource(&node().condition("MemoryPressure", true).build()).unwrap();
        assert_eq!(p.problems().count(), 1);

        let j = JobSummary::from_resource(&job().complete().completions(3).build()).unwrap();
        assert_eq!(j.completions_display(), "3/3");
        assert_eq!(j.status.as_str(), "Complete");
        let f = JobSummary::from_resource(&job().failed().build()).unwrap();
        assert_eq!(f.status.as_str(), "Failed");
        assert_eq!(JobSummary::from_resource(&job().build()).unwrap().active, 1);
    }

    #[test]
    fn resource_builder_builds_any_kind() {
        let cr = resource("test.oxikube.dev/v1", "Widget")
            .name("w1")
            .field("spec", json!({"size": "large"}))
            .build();
        assert_eq!(&*cr.kind.group, "test.oxikube.dev");
        assert_eq!(cr.get_str("/spec/size"), Some("large"));
        let ns = resource("v1", "Namespace")
            .name("demo")
            .cluster_scoped()
            .build();
        assert_eq!(ns.namespace(), None);
    }
}
