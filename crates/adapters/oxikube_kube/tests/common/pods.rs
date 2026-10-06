//! Image-pull-aware pod waits (E04-B01).
//!
//! `cargo xtask kind-up` pulls every test image into the nodes (`oxikube_testkit::images`), so a
//! pod normally starts in a second or two. A wait that must also survive a cold node, a slow
//! registry or a throttled scheduler would otherwise need a deadline long enough for the worst
//! case, which hides real hangs. These waits keep two clocks instead:
//!
//! - the **start budget** (the caller's deadline) runs only while the pod is not pulling an image;
//! - the **pull budget** ([`PULL_DEADLINE`]) runs only while it is: a container waiting in
//!   `ErrImagePull` / `ImagePullBackOff`, or in `ContainerCreating` with a `Pulling` event that has
//!   no matching `Pulled` yet.
//!
//! A permanently unusable image (`InvalidImageName`, `ErrImageNeverPull`) fails at once. Every
//! failure names the image, the containers' waiting reasons and the pod's events, so a CI log
//! shows what the cluster did without a rerun.

use std::fmt;
use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::{ContainerStatus, Event, Pod};
use kube::api::ListParams;
use kube::{Api, Client};

use super::{DEADLINE, POLL};

/// How long a pod may spend pulling images before the wait gives up. Generous because a pull is
/// network-bound, but finite: a registry that never answers is a failure, not a long wait.
pub const PULL_DEADLINE: Duration = Duration::from_secs(180);

/// Waiting reasons that mean the kubelet is (still) trying to get the image.
const PULL_REASONS: [&str; 2] = ["ErrImagePull", "ImagePullBackOff"];
/// Waiting reasons that no retry will fix.
const PERMANENT_REASONS: [&str; 3] = ["InvalidImageName", "ErrImageNeverPull", "ImageInspectError"];
/// Waiting reasons of a container the kubelet is preparing, where a pull may be in flight.
const PREPARING_REASONS: [&str; 2] = ["ContainerCreating", "PodInitializing"];

/// Why a pod wait failed.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    /// The pod did not reach the state within its start budget while not pulling.
    NotReached,
    /// The pod spent the whole pull budget pulling images.
    PullTooSlow,
    /// An image can never be used (invalid name, never-pull policy).
    BadImage,
}

/// A pod wait that failed, with everything needed to read the failure in a CI log.
#[derive(Debug)]
pub struct PodWaitError {
    /// Which failure.
    pub failure: Failure,
    /// What the wait was for, e.g. `pod web ready`.
    pub what: String,
    /// Time spent outside image pulls.
    pub active: Duration,
    /// Time spent pulling images.
    pub pulling: Duration,
    /// The pod's container states and events.
    pub diagnostics: String,
}

impl fmt::Display for PodWaitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let verdict = match self.failure {
            Failure::NotReached => "not reached",
            Failure::PullTooSlow => "not reached: the image pull took too long",
            Failure::BadImage => "not reached: an image cannot be used",
        };
        write!(
            f,
            "{} {verdict} (waited {:?} outside image pulls and {:?} pulling)\n{}",
            self.what, self.active, self.pulling, self.diagnostics
        )
    }
}

impl std::error::Error for PodWaitError {}

/// The two clocks of a wait. Pure, so the accounting is unit-tested without a cluster.
#[derive(Debug)]
pub struct Budget {
    start_limit: Duration,
    pull_limit: Duration,
    /// Time charged to the start budget.
    pub active: Duration,
    /// Time charged to the pull budget.
    pub pulling: Duration,
}

impl Budget {
    /// A budget of `start_limit` outside pulls and `pull_limit` inside them.
    pub fn new(start_limit: Duration, pull_limit: Duration) -> Self {
        Self {
            start_limit,
            pull_limit,
            active: Duration::ZERO,
            pulling: Duration::ZERO,
        }
    }

    /// Charges `elapsed` to the pull clock when `pulling`, else to the start clock; the failure
    /// when that clock is now spent.
    pub fn charge(&mut self, elapsed: Duration, pulling: bool) -> Option<Failure> {
        if pulling {
            self.pulling += elapsed;
            (self.pulling >= self.pull_limit).then_some(Failure::PullTooSlow)
        } else {
            self.active += elapsed;
            (self.active >= self.start_limit).then_some(Failure::NotReached)
        }
    }
}

fn statuses(pod: &Pod) -> impl Iterator<Item = &ContainerStatus> {
    let status = pod.status.as_ref();
    status
        .and_then(|s| s.init_container_statuses.as_ref())
        .into_iter()
        .flatten()
        .chain(
            status
                .and_then(|s| s.container_statuses.as_ref())
                .into_iter()
                .flatten(),
        )
}

fn waiting_reason(status: &ContainerStatus) -> Option<&str> {
    status.state.as_ref()?.waiting.as_ref()?.reason.as_deref()
}

/// Whether every container of `pod` is running or has run, so there is a log to read.
pub fn started(pod: &Pod) -> bool {
    let containers = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_deref())
        .unwrap_or_default();
    !containers.is_empty()
        && containers.iter().all(|s| {
            s.state
                .as_ref()
                .is_some_and(|st| st.running.is_some() || st.terminated.is_some())
        })
}

/// Whether `pod` has condition `Ready=True`.
pub fn ready(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|s| s.conditions.as_ref())
        .is_some_and(|c| c.iter().any(|c| c.type_ == "Ready" && c.status == "True"))
}

/// The first container whose image can never be used, as `container: Reason: message`.
pub fn bad_image(pod: &Pod) -> Option<String> {
    statuses(pod)
        .find(|s| waiting_reason(s).is_some_and(|r| PERMANENT_REASONS.contains(&r)))
        .map(describe_waiting)
}

fn describe_waiting(status: &ContainerStatus) -> String {
    let waiting = status.state.as_ref().and_then(|s| s.waiting.as_ref());
    format!(
        "{}: {}: {}",
        status.name,
        waiting.and_then(|w| w.reason.as_deref()).unwrap_or("?"),
        waiting.and_then(|w| w.message.as_deref()).unwrap_or("")
    )
}

/// The images the kubelet announced pulling (`Pulling image "X"`) and has not reported pulled
/// (`Pulled`: either `Successfully pulled image "X"` or `Container image "X" already present`).
pub fn images_being_pulled(events: &[Event]) -> Vec<String> {
    let quoted = |e: &Event| {
        let message = e.message.as_deref()?;
        let start = message.find('"')? + 1;
        let end = start + message[start..].find('"')?;
        Some(message[start..end].to_owned())
    };
    let pulled: Vec<String> = events
        .iter()
        .filter(|e| e.reason.as_deref() == Some("Pulled"))
        .filter_map(quoted)
        .collect();
    let mut pulling: Vec<String> = events
        .iter()
        .filter(|e| e.reason.as_deref() == Some("Pulling"))
        .filter_map(quoted)
        .filter(|image| !pulled.contains(image))
        .collect();
    pulling.dedup();
    pulling
}

/// Whether the pod is waiting on an image: a failing or backing-off pull, or a container being
/// created while `events` show a pull in flight.
pub fn is_pulling(pod: &Pod, events: &[Event]) -> bool {
    let mut preparing = false;
    for reason in statuses(pod).filter_map(waiting_reason) {
        if PULL_REASONS.contains(&reason) {
            return true;
        }
        preparing |= PREPARING_REASONS.contains(&reason);
    }
    preparing && !images_being_pulled(events).is_empty()
}

async fn pod_events(client: &Client, namespace: &str, name: &str) -> Vec<Event> {
    let params = ListParams::default().fields(&format!("involvedObject.name={name}"));
    let mut events = Api::<Event>::namespaced(client.clone(), namespace)
        .list(&params)
        .await
        .map(|l| l.items)
        .unwrap_or_default();
    events.sort_by_key(|e| {
        e.last_timestamp
            .clone()
            .or(e.metadata.creation_timestamp.clone())
    });
    events
}

/// The pod's container states and events, one line each. Reasons and messages written by the
/// kubelet and controllers; no object payloads.
pub fn diagnostics(pod: Option<&Pod>, events: &[Event]) -> String {
    let mut lines = Vec::new();
    match pod {
        None => lines.push("pod: not found".to_owned()),
        Some(pod) => {
            let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
            lines.push(format!("pod phase: {}", phase.unwrap_or("?")));
            // A pod the scheduler has not placed yet looks like a slow start; say so.
            match pod.spec.as_ref().and_then(|s| s.node_name.as_deref()) {
                Some(node) => lines.push(format!("scheduled to node {node}")),
                None => lines.push("not scheduled: no node assigned yet".to_owned()),
            }
            let conditions = pod
                .status
                .iter()
                .flat_map(|s| s.conditions.iter().flatten());
            for c in conditions.filter(|c| c.status != "True") {
                lines.push(format!(
                    "condition {}={}: {} {}",
                    c.type_,
                    c.status,
                    c.reason.as_deref().unwrap_or(""),
                    c.message.as_deref().unwrap_or("")
                ));
            }
            for status in statuses(pod) {
                match waiting_reason(status) {
                    Some(_) => lines.push(format!("container {}", describe_waiting(status))),
                    None => {
                        lines.push(format!("container {}: {}", status.name, state_name(status)))
                    }
                }
            }
        }
    }
    lines.push(format!("events ({}):", events.len()));
    for e in events {
        lines.push(format!(
            "  {} {}: {}",
            e.type_.as_deref().unwrap_or("?"),
            e.reason.as_deref().unwrap_or("?"),
            e.message.as_deref().unwrap_or("")
        ));
    }
    lines.join("\n")
}

fn state_name(status: &ContainerStatus) -> &'static str {
    match status.state.as_ref() {
        Some(s) if s.running.is_some() => "running",
        Some(s) if s.terminated.is_some() => "terminated",
        _ => "no state yet",
    }
}

/// Polls pod `name` until `reached`, on the two clocks of [`Budget`].
pub async fn wait_pod(
    client: &Client,
    namespace: &str,
    name: &str,
    what: &str,
    mut budget: Budget,
    reached: fn(&Pod) -> bool,
) -> Result<(), PodWaitError> {
    let pods = Api::<Pod>::namespaced(client.clone(), namespace);
    loop {
        let tick = Instant::now();
        let pod = pods.get_opt(name).await.ok().flatten();
        let mut events = Vec::new();
        let mut verdict = None;
        let mut pulling = false;
        if let Some(pod) = &pod {
            if reached(pod) {
                return Ok(());
            }
            if bad_image(pod).is_some() {
                verdict = Some(Failure::BadImage);
            } else if statuses(pod)
                .filter_map(waiting_reason)
                .any(|r| PULL_REASONS.contains(&r) || PREPARING_REASONS.contains(&r))
            {
                events = pod_events(client, namespace, name).await;
                pulling = is_pulling(pod, &events);
            }
        }
        tokio::time::sleep(POLL).await;
        verdict = verdict.or_else(|| budget.charge(tick.elapsed(), pulling));
        if let Some(failure) = verdict {
            if events.is_empty() {
                events = pod_events(client, namespace, name).await;
            }
            let latest = pods.get_opt(name).await.ok().flatten();
            return Err(PodWaitError {
                failure,
                what: what.to_owned(),
                active: budget.active,
                pulling: budget.pulling,
                diagnostics: diagnostics(latest.as_ref().or(pod.as_ref()), &events),
            });
        }
    }
}

/// Waits until every container of `name` is running or has run, then returns; panics with the
/// diagnostics otherwise. Start budget [`DEADLINE`].
pub async fn wait_started(client: &Client, namespace: &str, name: &str) {
    let what = format!("pod {name} started");
    let budget = Budget::new(DEADLINE, PULL_DEADLINE);
    if let Err(e) = wait_pod(client, namespace, name, &what, budget, started).await {
        panic!("{e}");
    }
}

/// Waits until the pod `name` is Ready; panics with the diagnostics otherwise. Start budget
/// of two minutes: readiness includes probes and a throttled scheduler.
pub async fn wait_ready(client: &Client, namespace: &str, name: &str) {
    let what = format!("pod {name} ready");
    let budget = Budget::new(Duration::from_secs(120), PULL_DEADLINE);
    if let Err(e) = wait_pod(client, namespace, name, &what, budget, ready).await {
        panic!("{e}");
    }
}
