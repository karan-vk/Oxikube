//! [`Fleet`]: the pods the selector matches and the streams opened for their containers.
//!
//! The watch's deltas go in; out come the streams to start (never more than `logs.max_streams`
//! at once), the pod events (`Added`, `Ended`) and the bookkeeping the viewer reads. The fleet
//! starts a stream at most once per container of a pod (a stream that ended is not reopened here:
//! following the replacements of a restarted pod is E08-S07's), and a pod deleted and recreated
//! under the same name is a new pod with new streams.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_ports::{Delta, DeltaBatch};

use super::pods::PodState;
use super::sources::{PodChange, SourceId, SourceInfo, SourceState};
use super::view::AggShared;
use crate::store::TaskGuard;

/// What to read of the pods that come and go.
#[derive(Clone)]
pub(super) struct Reading {
    /// Only containers of this name.
    pub container: Option<String>,
    /// The previous container instance (so only restarted containers have a log).
    pub previous: bool,
    /// A read that does not follow: `logs.max_streams` bounds the containers read in all, not
    /// only the ones read at once (a stream that ended frees no slot for a skipped pod).
    pub finite: bool,
}

/// See the [module docs](self).
pub(super) struct Fleet {
    reading: Reading,
    pods: BTreeMap<Arc<str>, PodState>,
    /// Streams by (pod uid, container name): the guard that aborts the stream's task when
    /// dropped, `None` once the stream ended.
    streams: HashMap<(Arc<str>, Arc<str>), Option<TaskGuard>>,
    /// The pod uid and container of each stream id.
    owners: HashMap<SourceId, (Arc<str>, Arc<str>)>,
    /// Whether the first list was received: pods in it are the baseline, not "added".
    baseline: bool,
    /// Uids whose `Ended` event was sent.
    ended: HashSet<Arc<str>>,
    next_id: SourceId,
}

impl Fleet {
    pub fn new(reading: Reading) -> Self {
        Self {
            reading,
            pods: BTreeMap::new(),
            streams: HashMap::new(),
            owners: HashMap::new(),
            baseline: false,
            ended: HashSet::new(),
            next_id: 0,
        }
    }

    /// Streams being read now.
    pub fn live(&self) -> usize {
        self.streams.values().flatten().count()
    }

    /// Applies one batch of the pod watch (or the one list a read that does not follow makes).
    pub fn apply(&mut self, batch: DeltaBatch<Resource>, agg: &AggShared) {
        for delta in batch {
            match delta {
                Delta::Restarted(items) => self.restart(items, agg),
                Delta::Applied(pod) => {
                    let state = self.state_of(&pod);
                    self.upsert(state, agg);
                }
                Delta::Deleted(pod) => self.deleted(pod.name(), agg),
            }
        }
        self.baseline = true;
    }

    fn state_of(&self, pod: &Resource) -> PodState {
        PodState::of(
            pod,
            self.reading.container.as_deref(),
            self.reading.previous,
        )
    }

    fn restart(&mut self, items: Vec<Resource>, agg: &AggShared) {
        let current: BTreeMap<Arc<str>, PodState> = items
            .iter()
            .map(|pod| {
                let state = self.state_of(pod);
                (state.name.clone(), state)
            })
            .collect();
        let gone: Vec<Arc<str>> = self
            .pods
            .keys()
            .filter(|name| !current.contains_key(*name))
            .cloned()
            .collect();
        for name in gone {
            self.deleted(&name, agg);
        }
        for state in current.into_values() {
            self.upsert(state, agg);
        }
    }

    /// Records `state`: an update of a known pod, a pod that is new, or (same name, new uid) a
    /// pod that was deleted and recreated.
    fn upsert(&mut self, state: PodState, agg: &AggShared) {
        match self.pods.get(&state.name) {
            Some(known) if known.uid != state.uid => {
                self.deleted(&state.name.clone(), agg);
                self.added(state, agg);
            }
            Some(_) => {
                self.pods.insert(state.name.clone(), state);
            }
            None => self.added(state, agg),
        }
    }

    fn added(&mut self, state: PodState, agg: &AggShared) {
        if self.baseline {
            agg.push_event(state.name.clone(), PodChange::Added);
        }
        self.pods.insert(state.name.clone(), state);
    }

    fn deleted(&mut self, name: &str, agg: &AggShared) {
        if let Some(pod) = self.pods.remove(name) {
            self.announce_ended(&pod, agg);
        }
    }

    fn announce_ended(&mut self, pod: &PodState, agg: &AggShared) {
        if self.ended.insert(pod.uid.clone()) {
            agg.push_event(pod.name.clone(), PodChange::Ended);
        }
    }

    /// Opens a stream for every streamable container that has none, up to `cap` live streams
    /// (up to `cap` streams in all for a finite read), pods in name order. `start` spawns the stream's task and returns its guard. Publishes the
    /// number of pods left out by the cap. Returns the ids of the streams opened.
    pub fn reconcile(
        &mut self,
        cap: usize,
        agg: &AggShared,
        mut start: impl FnMut(SourceId, &PodState, &Arc<str>) -> TaskGuard,
    ) -> Vec<SourceId> {
        let mut opened = Vec::new();
        let mut live = if self.reading.finite {
            self.streams.len()
        } else {
            self.live()
        };
        let mut skipped: HashSet<&str> = HashSet::new();
        for pod in self.pods.values() {
            for container in pod.containers.iter().filter(|c| c.streamable) {
                let key = (pod.uid.clone(), container.name.clone());
                if self.streams.contains_key(&key) {
                    continue;
                }
                if live >= cap {
                    skipped.insert(&pod.name);
                    continue;
                }
                let id = self.next_id;
                self.next_id += 1;
                agg.add_source(SourceInfo {
                    id,
                    pod: pod.name.clone(),
                    container: container.name.clone(),
                    state: SourceState::Streaming,
                });
                let guard = start(id, pod, &container.name);
                self.owners.insert(id, key.clone());
                self.streams.insert(key, Some(guard));
                live += 1;
                opened.push(id);
            }
        }
        agg.set_pod_counts(self.pods.len(), skipped.len());
        opened
    }

    /// A stream stopped: records it, and announces the pod's end when it was the last of the
    /// pod's streams.
    pub fn stream_ended(&mut self, id: SourceId, state: SourceState, agg: &AggShared) {
        agg.set_source_state(id, state);
        let Some(key) = self.owners.get(&id) else {
            return;
        };
        if let Some(guard) = self.streams.get_mut(key) {
            *guard = None;
        }
        let uid = key.0.clone();
        let Some(pod) = self.pods.values().find(|pod| pod.uid == uid) else {
            return;
        };
        let all_done = pod.containers.iter().filter(|c| c.streamable).all(|c| {
            self.streams
                .get(&(uid.clone(), c.name.clone()))
                .is_some_and(Option::is_none)
        });
        if all_done && self.ended.insert(uid) {
            agg.push_event(pod.name.clone(), PodChange::Ended);
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::view::AggregateView;
    use super::*;

    fn pod(name: &str, uid: &str, running: bool) -> Resource {
        let state = if running {
            json!({"running": {}})
        } else {
            json!({"waiting": {"reason": "ContainerCreating"}})
        };
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": name, "namespace": "shop", "uid": uid},
            "spec": {"containers": [{"name": "app"}]},
            "status": {"containerStatuses": [{"name": "app", "state": state}]}
        }))
        .unwrap()
    }

    fn batch(deltas: Vec<Delta<Resource>>) -> DeltaBatch<Resource> {
        DeltaBatch::from_deltas(deltas)
    }

    fn guard() -> TaskGuard {
        let spawner: Arc<dyn crate::store::Spawner> =
            Arc::new(|_: futures::future::BoxFuture<'static, ()>| {});
        crate::store::spawn_guarded(&spawner, async {})
    }

    fn fleet() -> Fleet {
        Fleet::new(Reading {
            container: None,
            previous: false,
            finite: false,
        })
    }

    #[test]
    fn the_first_list_is_the_baseline_and_later_pods_are_added() {
        let agg = Arc::new(AggShared::new("deployment/web".into()));
        let mut fleet = fleet();
        fleet.apply(
            batch(vec![Delta::Restarted(vec![pod("web-1", "u1", true)])]),
            &agg,
        );
        fleet.apply(batch(vec![Delta::Applied(pod("web-2", "u2", true))]), &agg);
        let events = AggregateView::new(agg).events_after(None);
        assert_eq!(events.len(), 1);
        assert_eq!(&*events[0].pod, "web-2");
        assert_eq!(events[0].change, PodChange::Added);
    }

    #[test]
    fn the_cap_leaves_pods_out_and_a_freed_slot_takes_the_next() {
        let agg = Arc::new(AggShared::new("deployment/web".into()));
        let mut fleet = fleet();
        fleet.apply(
            batch(vec![Delta::Restarted(vec![
                pod("web-1", "u1", true),
                pod("web-2", "u2", true),
                pod("web-3", "u3", true),
            ])]),
            &agg,
        );
        let opened = fleet.reconcile(2, &agg, |_, _, _| guard());
        assert_eq!(opened, [0, 1], "pods in name order");
        let view = AggregateView::new(agg.clone());
        assert_eq!(view.skipped_pods(), 1);
        fleet.stream_ended(0, SourceState::Ended, &agg);
        let opened = fleet.reconcile(2, &agg, |_, _, _| guard());
        assert_eq!(opened, [2]);
        assert_eq!(view.skipped_pods(), 0);
        assert_eq!(view.sources().len(), 3);
    }

    #[test]
    fn a_finite_read_opens_at_most_the_cap_in_all() {
        let agg = Arc::new(AggShared::new("deployment/web".into()));
        let mut fleet = Fleet::new(Reading {
            container: None,
            previous: false,
            finite: true,
        });
        fleet.apply(
            batch(vec![Delta::Restarted(vec![
                pod("web-1", "u1", true),
                pod("web-2", "u2", true),
                pod("web-3", "u3", true),
            ])]),
            &agg,
        );
        assert_eq!(fleet.reconcile(2, &agg, |_, _, _| guard()), [0, 1]);
        fleet.stream_ended(0, SourceState::Ended, &agg);
        assert!(
            fleet.reconcile(2, &agg, |_, _, _| guard()).is_empty(),
            "an ended stream does not free a slot"
        );
        assert_eq!(AggregateView::new(agg.clone()).skipped_pods(), 1);
        assert_eq!(fleet.reconcile(3, &agg, |_, _, _| guard()), [2]);
    }

    #[test]
    fn a_container_that_is_not_running_yet_is_read_when_the_pod_updates() {
        let agg = Arc::new(AggShared::new(String::new()));
        let mut fleet = fleet();
        fleet.apply(
            batch(vec![Delta::Restarted(vec![pod("web-1", "u1", false)])]),
            &agg,
        );
        assert!(fleet.reconcile(5, &agg, |_, _, _| guard()).is_empty());
        fleet.apply(batch(vec![Delta::Applied(pod("web-1", "u1", true))]), &agg);
        assert_eq!(fleet.reconcile(5, &agg, |_, _, _| guard()), [0]);
        assert!(
            fleet.reconcile(5, &agg, |_, _, _| guard()).is_empty(),
            "a container is opened once"
        );
    }

    #[test]
    fn pods_with_no_readable_container_still_count_as_matched() {
        let agg = Arc::new(AggShared::new(String::new()));
        let mut fleet = fleet();
        fleet.apply(
            batch(vec![Delta::Restarted(vec![pod("web-1", "u1", false)])]),
            &agg,
        );
        fleet.reconcile(5, &agg, |_, _, _| guard());
        let view = AggregateView::new(agg.clone());
        assert_eq!(view.matched_pods(), 1);
        assert!(view.sources().is_empty());
        fleet.apply(batch(vec![Delta::Deleted(pod("web-1", "u1", false))]), &agg);
        fleet.reconcile(5, &agg, |_, _, _| guard());
        assert_eq!(view.matched_pods(), 0);
    }

    #[test]
    fn a_pod_recreated_under_the_same_name_gets_new_streams() {
        let agg = Arc::new(AggShared::new(String::new()));
        let mut fleet = fleet();
        fleet.apply(
            batch(vec![Delta::Restarted(vec![pod("web-0", "u1", true)])]),
            &agg,
        );
        fleet.reconcile(5, &agg, |_, _, _| guard());
        fleet.stream_ended(0, SourceState::Ended, &agg);
        fleet.apply(batch(vec![Delta::Deleted(pod("web-0", "u1", true))]), &agg);
        fleet.apply(batch(vec![Delta::Applied(pod("web-0", "u2", true))]), &agg);
        assert_eq!(fleet.reconcile(5, &agg, |_, _, _| guard()), [1]);
        let events = AggregateView::new(agg).events_after(None);
        let changes: Vec<_> = events.iter().map(|e| e.change).collect();
        assert_eq!(changes, [PodChange::Ended, PodChange::Added]);
    }

    #[test]
    fn a_pod_ends_when_its_last_stream_does_and_only_once() {
        let agg = Arc::new(AggShared::new(String::new()));
        let mut fleet = fleet();
        fleet.apply(
            batch(vec![Delta::Restarted(vec![pod("web-1", "u1", true)])]),
            &agg,
        );
        fleet.reconcile(5, &agg, |_, _, _| guard());
        fleet.stream_ended(0, SourceState::Ended, &agg);
        fleet.apply(batch(vec![Delta::Deleted(pod("web-1", "u1", true))]), &agg);
        let events = AggregateView::new(agg).events_after(None);
        assert_eq!(events.len(), 1, "one Ended event for the pod");
        assert_eq!(events[0].change, PodChange::Ended);
    }
}
