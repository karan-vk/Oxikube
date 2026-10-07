//! The vocabulary of an aggregate's streams: [`SourceInfo`] (one container of one pod),
//! [`PodEvent`] (the pod set changed) and [`HiddenSources`] (what the user switched off).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::logs::LogFailure;

/// The id of one stream of an aggregate, assigned in the order the streams were opened. It is
/// the first tiebreak of lines with equal server timestamps, so it is stable for a session.
pub type SourceId = u32;

/// How one stream is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceState {
    /// Being read.
    Streaming,
    /// The server closed it: the container stopped, the pod went away, or the read was complete.
    Ended,
    /// It could not be opened or broke. Other streams are not affected.
    Failed(LogFailure),
}

impl SourceState {
    /// Whether the stream is still being read.
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Streaming)
    }
}

/// One stream of an aggregate: one container of one pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    /// The stream's id.
    pub id: SourceId,
    /// Pod name.
    pub pod: Arc<str>,
    /// Container name.
    pub container: Arc<str>,
    /// How it is doing.
    pub state: SourceState,
}

/// What happened to the set of pods a selector matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PodChange {
    /// A pod that matches the selector appeared after the view opened.
    Added,
    /// A pod was deleted, or every stream of it has ended.
    Ended,
}

/// One change of the pod set: the view's "pod web-7d9 added" / "pod web-4c1 ended" banner, and the
/// event churn following (E08-S07) reacts to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PodEvent {
    /// Position in the aggregate's event log: 0 for the first, then +1 for each.
    pub seq: u64,
    /// The pod.
    pub pod: Arc<str>,
    /// What happened to it.
    pub change: PodChange,
}

/// The pods and containers the user switched off. A hidden source keeps streaming; its lines stay
/// in the buffer and are left out of the view, so switching it back on shows them again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenSources {
    pods: HashSet<Arc<str>>,
    containers: HashMap<Arc<str>, HashSet<Arc<str>>>,
}

impl HiddenSources {
    /// Whether nothing is hidden.
    pub fn is_empty(&self) -> bool {
        self.pods.is_empty() && self.containers.is_empty()
    }

    /// Whether the lines of `container` of `pod` are hidden. Cheap: two hash lookups of short
    /// strings, so it can run for every line a view appends.
    pub fn is_hidden(&self, pod: &str, container: &str) -> bool {
        self.pods.contains(pod)
            || self
                .containers
                .get(pod)
                .is_some_and(|hidden| hidden.contains(container))
    }

    /// Whether the whole pod is hidden.
    pub fn is_pod_hidden(&self, pod: &str) -> bool {
        self.pods.contains(pod)
    }

    /// Hides or shows one container of `pod`, or the whole pod when `container` is `None`.
    pub fn toggle(&mut self, pod: &str, container: Option<&str>) {
        match container {
            None => {
                if !self.pods.remove(pod) {
                    self.pods.insert(Arc::from(pod));
                }
            }
            Some(container) => {
                let hidden = self.containers.entry(Arc::from(pod)).or_default();
                if !hidden.remove(container) {
                    hidden.insert(Arc::from(container));
                }
                if hidden.is_empty() {
                    self.containers.remove(pod);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pod_or_one_of_its_containers_can_be_switched_off_and_on() {
        let mut hidden = HiddenSources::default();
        assert!(hidden.is_empty());
        hidden.toggle("web-1", Some("proxy"));
        assert!(hidden.is_hidden("web-1", "proxy"));
        assert!(!hidden.is_hidden("web-1", "app"));
        assert!(!hidden.is_hidden("web-2", "proxy"));
        hidden.toggle("web-1", None);
        assert!(hidden.is_hidden("web-1", "app") && hidden.is_pod_hidden("web-1"));
        hidden.toggle("web-1", None);
        hidden.toggle("web-1", Some("proxy"));
        assert!(hidden.is_empty(), "everything toggled back");
    }
}
