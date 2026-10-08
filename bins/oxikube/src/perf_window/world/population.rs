//! [`Population`]: the pods of one synthetic cluster, and what a churn tick does to them.
//!
//! The pods are spread round-robin over the namespaces, as `cargo xtask load-pods` spreads them,
//! and look like a working cluster's: most `Running` on one of a few nodes with an IP, some
//! `Pending`, some crash-looping with restarts. A churn tick is `load-pods --churn`'s: about 1 % of
//! the pods (a sliding window over the indices) are deleted and created again under the same name,
//! which a watch sees as `MODIFIED` (the deletion timestamp), `DELETED` and `ADDED`, with a
//! `Killing` and a `Scheduled` event for each in its namespace.

use jiff::{SignedDuration, Timestamp};
use oxikube_domain::Resource;
use oxikube_ports::{Delta, DeltaBatch};
use oxikube_testkit::pod;
use serde_json::{Value, json};

/// Namespace `i` of a synthetic cluster.
pub fn namespace_name(i: usize) -> String {
    format!("oxikube-load-{i}")
}

/// One synthetic cluster's pods.
#[derive(Debug)]
pub struct Population {
    namespaces: usize,
    pods: Vec<Resource>,
    /// How many times each pod was recreated (its uid changes each time).
    generation: Vec<u32>,
    version: u64,
    created: Timestamp,
}

impl Population {
    /// `count` pods over `namespaces` namespaces, created two days before `now`.
    pub fn new(count: usize, namespaces: usize, now: Timestamp) -> Self {
        let namespaces = namespaces.max(1);
        let created = now
            .checked_sub(SignedDuration::from_hours(49))
            .unwrap_or(now);
        let mut population = Self {
            namespaces,
            pods: Vec::with_capacity(count),
            generation: vec![0; count],
            version: 0,
            created,
        };
        for i in 0..count {
            let pod = population.build(i, Steady::of(i), created, false);
            population.pods.push(pod);
        }
        population
    }

    /// Every pod now, in `namespace` (all when `None`).
    pub fn pods(&self, namespace: Option<&str>) -> Vec<Resource> {
        self.pods
            .iter()
            .filter(|p| namespace.is_none_or(|ns| p.namespace() == Some(ns)))
            .cloned()
            .collect()
    }

    /// Number of pods.
    pub fn len(&self) -> usize {
        self.pods.len()
    }

    /// The namespaces the pods are in.
    pub fn namespaces(&self) -> usize {
        self.namespaces
    }

    /// Pods recycled per churn tick: 1 % of them, at least one (`load-pods`' step).
    pub fn churn_step(&self) -> usize {
        (self.pods.len() / 100).max(1)
    }

    /// Recycles the pods `first .. first + n` (wrapping) at `now`: the pod events (a `MODIFIED`
    /// with the deletion timestamp, the `DELETED`, the new pod's `ADDED`) and the event objects
    /// about them (`Killing` for the old pod, `Scheduled` for the new one).
    pub fn recycle(&mut self, first: usize, n: usize, now: Timestamp) -> Recycled {
        let mut pods = Vec::with_capacity(3 * n);
        let mut events = Vec::with_capacity(2 * n);
        if self.pods.is_empty() {
            return Recycled::default();
        }
        for k in 0..n {
            let i = (first + k) % self.pods.len();
            let old = self.build(i, Steady::of(i), self.created_of(i), true);
            pods.push(Delta::Applied(old.clone()));
            pods.push(Delta::Deleted(old.clone()));
            events.push(Delta::Applied(self.event(&old, "Killing", now)));
            self.generation[i] += 1;
            let new = self.build(i, Steady::Pending, now, false);
            events.push(Delta::Applied(self.event(&new, "Scheduled", now)));
            pods.push(Delta::Applied(new.clone()));
            self.pods[i] = new;
        }
        Recycled {
            pods: DeltaBatch::from_deltas(pods),
            events: DeltaBatch::from_deltas(events),
        }
    }

    fn created_of(&self, i: usize) -> Timestamp {
        self.pods
            .get(i)
            .and_then(|p| p.meta.creation)
            .unwrap_or(self.created)
    }

    fn next_version(&mut self) -> String {
        self.version += 1;
        self.version.to_string()
    }

    fn build(&mut self, i: usize, state: Steady, created: Timestamp, deleting: bool) -> Resource {
        let node = format!("worker-{}", i % 6);
        let mut builder = pod()
            .namespace(namespace_name(i % self.namespaces))
            .name(format!("load-{i:05}"))
            .uid(format!(
                "00000000-0000-4000-8000-{:06}{:06}",
                i, self.generation[i]
            ))
            .label("app", "oxikube-load")
            .label("pod-template-hash", format!("{:x}", i % 997))
            .image("registry.k8s.io/pause:3.10")
            .created(created.to_string());
        builder = match state {
            Steady::Running => builder.running().node(node).ip(format!(
                "10.244.{}.{}",
                (i / 250) % 250,
                i % 250 + 2
            )),
            Steady::Pending => builder.pending(),
            Steady::CrashLoop => builder
                .crash_loop()
                .restarts(u32::try_from(i % 40).unwrap_or(0) + 1)
                .node(node),
        };
        if deleting {
            builder = builder.terminating();
        }
        let mut pod = builder.build();
        pod.meta.resource_version = Some(self.next_version().into());
        pod
    }

    fn event(&mut self, about: &Resource, reason: &str, now: Timestamp) -> Resource {
        let name = format!("{}.{:x}", about.name(), self.version);
        let json = json!({
            "apiVersion": "v1",
            "kind": "Event",
            "metadata": {
                "name": name,
                "namespace": about.namespace(),
                "resourceVersion": self.next_version(),
                "creationTimestamp": now.to_string(),
            },
            "involvedObject": {
                "apiVersion": "v1",
                "kind": "Pod",
                "name": about.name(),
                "namespace": about.namespace(),
                "uid": about.meta.uid.as_deref(),
            },
            "reason": reason,
            "message": event_message(reason, about.name()),
            "type": "Normal",
            "count": 1,
            "firstTimestamp": now.to_string(),
            "lastTimestamp": now.to_string(),
            "source": { "component": "kubelet" },
        });
        // Every field above is valid for a `Resource`, so this cannot fail.
        Resource::from_json(json).unwrap_or_else(|_| about.clone())
    }
}

/// What one [`Population::recycle`] produced.
#[derive(Debug, Default)]
pub struct Recycled {
    /// The pod watch events.
    pub pods: DeltaBatch<Resource>,
    /// The `Event` objects about them.
    pub events: DeltaBatch<Resource>,
}

/// The state a pod settles in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Steady {
    Running,
    Pending,
    CrashLoop,
}

impl Steady {
    /// Most pods run; one in 25 is pending, one in 50 crash-loops.
    fn of(i: usize) -> Self {
        match i % 50 {
            7 => Steady::CrashLoop,
            3 | 29 => Steady::Pending,
            _ => Steady::Running,
        }
    }
}

fn event_message(reason: &str, pod: &str) -> String {
    match reason {
        "Killing" => format!("Stopping container {pod}"),
        _ => format!("Successfully assigned the pod {pod} to a node"),
    }
}

/// A metadata-only copy of `object`, as a `PartialObjectMetadata` watch delivers it.
pub fn metadata_only(object: &Resource) -> Resource {
    let json: Value = json!({
        "apiVersion": object.json["apiVersion"],
        "kind": object.json["kind"],
        "metadata": object.json["metadata"],
    });
    Resource::from_json(json).map_or_else(|_| object.clone(), Resource::into_partial)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Timestamp {
        "2026-10-08T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn pods_spread_over_namespaces_and_mostly_run() {
        let population = Population::new(1_000, 8, now());
        assert_eq!(population.len(), 1_000);
        assert_eq!(population.churn_step(), 10);
        assert_eq!(population.pods(Some("oxikube-load-3")).len(), 125);
        let pending = population
            .pods(None)
            .iter()
            .filter(|p| p.json["status"]["phase"] == "Pending")
            .count();
        assert_eq!(pending, 40);
    }

    #[test]
    fn a_recycle_deletes_and_recreates_under_the_same_name() {
        let mut population = Population::new(100, 4, now());
        let before = population.pods(None);
        let recycled = population.recycle(98, 3, now());
        // MODIFIED + DELETED + ADDED per pod, two events each.
        assert_eq!(recycled.pods.deltas.len(), 9);
        assert_eq!(recycled.events.deltas.len(), 6);
        let Delta::Applied(deleting) = &recycled.pods.deltas[0] else {
            panic!("a MODIFIED first");
        };
        assert_eq!(deleting.name(), "load-00098");
        assert!(deleting.json["metadata"]["deletionTimestamp"].is_string());
        assert!(matches!(&recycled.pods.deltas[1], Delta::Deleted(p) if p.name() == "load-00098"));
        // Wraps to the start.
        assert!(matches!(&recycled.pods.deltas[8], Delta::Applied(p) if p.name() == "load-00000"));
        let after = population.pods(None);
        assert_eq!(after.len(), 100);
        assert_ne!(after[0].meta.uid, before[0].meta.uid, "a new pod");
        assert_eq!(after[1].meta.uid, before[1].meta.uid, "untouched");
        let versions: std::collections::BTreeSet<_> = after
            .iter()
            .map(|p| p.meta.resource_version.clone())
            .collect();
        assert_eq!(versions.len(), 100, "every version is distinct");
    }

    #[test]
    fn metadata_only_keeps_identity_and_drops_the_spec() {
        let population = Population::new(1, 1, now());
        let full = &population.pods(None)[0];
        let partial = metadata_only(full);
        assert!(partial.is_partial());
        assert_eq!(partial.name(), full.name());
        assert!(partial.json.get("spec").is_none());
    }
}
