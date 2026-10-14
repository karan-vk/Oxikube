//! Events fixtures of the kind suites (E14-S10): objects that make the kubelet emit `Warning`
//! events, and [`EventArrivals`], the moment each event of a namespace reached a plain `kubectl`
//! watch, which a test compares with the moment the app showed it.
//!
//! | Fixture | What it is for |
//! |---|---|
//! | [`unpullable_pod`] | one pod whose image never pulls: `Failed` (`ErrImagePull`, `ImagePullBackOff`) Warnings within a second, then again on every back-off |
//! | [`unpullable_deployment`] | a Deployment of such pods: a burst of the same reason over several objects |
//! | [`EventArrivals`] | `kubectl get events --watch` on the test's namespace, timestamped per line |
//!
//! The image is [`UNPULLABLE_IMAGE`], under the reserved `.invalid` top-level domain: no node
//! ever pulls it, so it is deliberately absent from `fixtures/test-images.txt`. The pods use no
//! resources (they never start) and go with the test's namespace.

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::IntegrationError;
use super::pods::{MAIN, SUITE_LABEL};

/// An image no registry serves (`.invalid` is reserved, RFC 2606): the pull fails at once.
pub const UNPULLABLE_IMAGE: &str = "registry.invalid/oxikube/does-not-exist:1";

/// The reason the kubelet gives every `Warning` about an image it cannot pull.
pub const PULL_FAILED_REASON: &str = "Failed";

/// The reason of the marker event [`EventArrivals::start`] waits for.
const READY_REASON: &str = "OxiArrivalsReady";

fn unpullable_container() -> Value {
    json!({ "name": MAIN, "image": UNPULLABLE_IMAGE, "imagePullPolicy": "IfNotPresent" })
}

/// A pod `name` whose only container's image never pulls. It never starts and never restarts.
pub fn unpullable_pod(name: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": { "name": name, "labels": { SUITE_LABEL: "unpullable" } },
        "spec": {
            "restartPolicy": "Never",
            "terminationGracePeriodSeconds": 0,
            "containers": [unpullable_container()],
        },
    })
}

/// A Deployment `name` of `replicas` pods whose image never pulls: a burst of `Failed` Warnings
/// about as many pods within seconds.
pub fn unpullable_deployment(name: &str, replicas: u32) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": { "name": name, "labels": { SUITE_LABEL: "unpullable" } },
        "spec": {
            "replicas": replicas,
            "selector": { "matchLabels": { "oxikube.test/deployment": name } },
            "template": {
                "metadata": { "labels": { "oxikube.test/deployment": name, SUITE_LABEL: "unpullable" } },
                "spec": {
                    "terminationGracePeriodSeconds": 0,
                    "containers": [unpullable_container()],
                },
            },
        },
    })
}

/// Creates `manifest` (one object as JSON) in `namespace` with `kubectl create -f -`.
///
/// # Errors
///
/// [`IntegrationError::Kubectl`] when kubectl cannot be run or refuses the object.
pub fn create(context: &str, namespace: &str, manifest: &Value) -> Result<(), IntegrationError> {
    use std::io::Write as _;
    let mut child = Command::new("kubectl")
        .args(["--context", context, "-n", namespace, "create", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| IntegrationError::Kubectl(e.to_string()))?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(manifest.to_string().as_bytes())
        .map_err(|e| IntegrationError::Kubectl(e.to_string()))?;
    let out = child
        .wait_with_output()
        .map_err(|e| IntegrationError::Kubectl(e.to_string()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(IntegrationError::Kubectl(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ))
    }
}

/// One event as the `kubectl` watch printed it, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    /// When the line was read.
    pub at: Instant,
    /// `metadata.uid`.
    pub uid: String,
    /// `Normal` or `Warning`.
    pub event_type: String,
    /// The reason (`Failed`).
    pub reason: String,
    /// The involved object's kind (`Pod`).
    pub kind: String,
    /// The involved object's name.
    pub name: String,
}

impl Arrival {
    /// Whether this is a `Warning` about the `kind` object `name`.
    pub fn is_warning_about(&self, kind: &str, name: &str) -> bool {
        self.event_type == "Warning" && self.kind == kind && self.name == name
    }

    fn parse(line: &str, at: Instant) -> Option<Self> {
        let mut fields = line.split_whitespace();
        let mut next = || fields.next().map(str::to_owned);
        Some(Self {
            at,
            uid: next()?,
            event_type: next()?,
            reason: next()?,
            kind: next()?,
            name: next()?,
        })
    }
}

/// `kubectl get events --watch` on one namespace, each event's first arrival timestamped on
/// this machine's monotonic clock. The app and this watch read the same API server, so the gap
/// between this arrival and the app's is the app's own latency (the kubelet's timing drops out).
/// Stopped (the process killed) on drop.
pub struct EventArrivals {
    child: Child,
    seen: Arc<Mutex<Vec<Arrival>>>,
    reader: Option<JoinHandle<()>>,
}

impl EventArrivals {
    /// Starts the watch on `namespace` and returns once it is live: it creates a `Normal`
    /// marker event about the namespace and waits (at most `deadline`) until the watch prints
    /// it, so nothing created afterwards can be missed.
    ///
    /// # Errors
    ///
    /// [`IntegrationError::Kubectl`] when kubectl cannot be run, the marker cannot be created
    /// or the watch does not print it in time.
    pub fn start(
        context: &str,
        namespace: &str,
        deadline: Duration,
    ) -> Result<Self, IntegrationError> {
        let columns = "custom-columns=UID:.metadata.uid,TYPE:.type,REASON:.reason,\
                       KIND:.involvedObject.kind,NAME:.involvedObject.name";
        let mut child = Command::new("kubectl")
            .args(["--context", context, "-n", namespace])
            .args(["get", "events", "--watch", "--no-headers", "-o", columns])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| IntegrationError::Kubectl(e.to_string()))?;
        let stdout = child.stdout.take().expect("piped stdout");
        let seen = Arc::new(Mutex::new(Vec::<Arrival>::new()));
        let sink = seen.clone();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Some(arrival) = Arrival::parse(&line, Instant::now()) else {
                    continue;
                };
                let mut seen = sink.lock();
                if !seen.iter().any(|a| a.uid == arrival.uid) {
                    seen.push(arrival);
                }
            }
        });
        let mut arrivals = Self {
            child,
            seen,
            reader: Some(reader),
        };
        create(context, namespace, &marker(namespace))?;
        arrivals
            .wait_for(deadline, |a| a.reason == READY_REASON)
            .ok_or_else(|| {
                IntegrationError::Kubectl(format!(
                    "the events watch on {namespace} printed nothing in {deadline:?}"
                ))
            })?;
        arrivals.seen.lock().retain(|a| a.reason != READY_REASON);
        Ok(arrivals)
    }

    /// Every event seen so far (its first arrival), oldest first. The marker is not listed.
    pub fn arrivals(&self) -> Vec<Arrival> {
        self.seen.lock().clone()
    }

    /// The first arrival that matches `pred`, if any yet.
    pub fn first(&self, pred: impl Fn(&Arrival) -> bool) -> Option<Arrival> {
        self.seen.lock().iter().find(|a| pred(a)).cloned()
    }

    /// Waits (polling every 10 ms, at most `deadline`) for an arrival that matches `pred`.
    pub fn wait_for(
        &mut self,
        deadline: Duration,
        pred: impl Fn(&Arrival) -> bool,
    ) -> Option<Arrival> {
        let started = Instant::now();
        loop {
            if let Some(found) = self.first(&pred) {
                return Some(found);
            }
            if started.elapsed() >= deadline || self.child.try_wait().ok().flatten().is_some() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The arrivals as text (`type reason kind/name uid`, one per line), for a failure message.
    pub fn describe(&self) -> String {
        let seen = self.seen.lock();
        if seen.is_empty() {
            return "(no events seen by the kubectl watch)".to_owned();
        }
        seen.iter()
            .map(|a| {
                format!(
                    "{} {} {}/{} {}",
                    a.event_type, a.reason, a.kind, a.name, a.uid
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Drop for EventArrivals {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// The `Normal` event [`EventArrivals::start`] creates to know its watch is live.
fn marker(namespace: &str) -> Value {
    json!({
        "apiVersion": "v1",
        "kind": "Event",
        "metadata": { "generateName": "oxi-arrivals-", "namespace": namespace },
        "involvedObject": {
            "apiVersion": "v1", "kind": "Namespace", "name": namespace, "namespace": namespace
        },
        "reason": READY_REASON,
        "message": "the test's events watch is live",
        "type": "Normal",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integration::{TestNamespace, test_context};

    #[test]
    fn a_watch_line_parses_into_an_arrival() {
        let at = Instant::now();
        let a = Arrival::parse("c406  Warning   Failed   Pod   bad", at).expect("parsed");
        assert!(a.is_warning_about("Pod", "bad"));
        assert_eq!((a.uid.as_str(), a.reason.as_str()), ("c406", "Failed"));
        assert!(Arrival::parse("only three fields", at).is_none());
    }

    #[test]
    fn the_fixtures_name_the_unpullable_image() {
        let pod = unpullable_pod("p");
        assert_eq!(pod["spec"]["containers"][0]["image"], UNPULLABLE_IMAGE);
        let deployment = unpullable_deployment("d", 4);
        assert_eq!(deployment["spec"]["replicas"], 4);
        assert_eq!(
            deployment["spec"]["selector"]["matchLabels"],
            deployment["spec"]["template"]["metadata"]["labels"]["oxikube.test/deployment"]
                .as_str()
                .map(|v| json!({ "oxikube.test/deployment": v }))
                .expect("a label")
        );
    }

    /// Runs only with a cluster (`OXIKUBE_TEST_CONTEXT=kind-oxikube`); skips otherwise.
    #[test]
    fn an_unpullable_pod_warns_and_the_watch_sees_it() {
        let Some(ctx) = test_context() else { return };
        let ns = TestNamespace::create(&ctx).expect("namespace");
        let mut arrivals =
            EventArrivals::start(&ctx, ns.name(), Duration::from_secs(30)).expect("the watch");
        create(&ctx, ns.name(), &unpullable_pod("bad")).expect("the pod");
        let warning = arrivals.wait_for(Duration::from_secs(60), |a| {
            a.is_warning_about("Pod", "bad") && a.reason == PULL_FAILED_REASON
        });
        assert!(
            warning.is_some(),
            "{}\n{}",
            arrivals.describe(),
            ns.events()
        );
    }
}
