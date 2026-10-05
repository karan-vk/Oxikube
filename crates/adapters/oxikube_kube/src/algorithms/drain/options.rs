//! The drain's inputs and outputs: options, the plan's pod types and the progress events.

use std::fmt;
use std::time::Duration;

/// How a drain behaves. [`Default`] is `kubectl drain` with no flags, plus a bounded wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainOptions {
    /// Skip DaemonSet-managed pods (`--ignore-daemonsets`). Without it such a pod blocks the
    /// drain, because a DaemonSet recreates it at once.
    pub ignore_daemonsets: bool,
    /// Evict pods that use `emptyDir` volumes, whose data is lost (`--delete-emptydir-data`).
    /// Without it such a pod blocks the drain.
    pub delete_emptydir_data: bool,
    /// Evict pods no controller manages and nothing will recreate (`--force`). Without it such
    /// a pod blocks the drain.
    pub force: bool,
    /// Grace period for each evicted pod, in seconds (`--grace-period`); `None` uses the pod's
    /// own.
    pub grace_period_secs: Option<u32>,
    /// The whole drain's deadline, from the first eviction. A pod still blocked or still
    /// terminating then is reported as failed. Default five minutes.
    pub timeout: Duration,
    /// The first wait after an eviction a budget refuses; it doubles up to
    /// [`retry_max`](Self::retry_max). Default one second.
    pub retry_initial: Duration,
    /// The longest wait between eviction attempts. Default ten seconds.
    pub retry_max: Duration,
    /// How often a pod is checked while waiting for it to go. Default one second.
    pub poll_interval: Duration,
    /// How many pods are evicted at once (at least one). Default four.
    pub concurrency: usize,
    /// Plan only: report what would be evicted and change nothing.
    pub dry_run: bool,
}

impl Default for DrainOptions {
    fn default() -> Self {
        Self {
            ignore_daemonsets: false,
            delete_emptydir_data: false,
            force: false,
            grace_period_secs: None,
            timeout: Duration::from_secs(300),
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(10),
            poll_interval: Duration::from_secs(1),
            concurrency: 4,
            dry_run: false,
        }
    }
}

/// A pod named by the drain: its identity for the eviction and for "is it gone".
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PodRef {
    /// The pod's namespace.
    pub namespace: String,
    /// The pod's name.
    pub name: String,
    /// The pod's uid when known; evictions carry it as a precondition.
    pub uid: Option<String>,
}

impl fmt::Display for PodRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.namespace, self.name)
    }
}

/// Why the drain leaves a pod alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// A mirror pod of a static pod: the kubelet owns it and an eviction cannot remove it.
    Mirror,
    /// Managed by a DaemonSet, and the drain ignores those.
    DaemonSet,
}

/// Why a pod stops the drain before anything is changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// Managed by a DaemonSet; set `ignore_daemonsets`.
    DaemonSet,
    /// Uses an `emptyDir` volume; set `delete_emptydir_data`.
    LocalStorage,
    /// No controller manages it; set `force`.
    Unmanaged,
}

impl BlockReason {
    /// What stops the drain, and the option that allows it.
    pub fn explain(self) -> &'static str {
        match self {
            Self::DaemonSet => "is managed by a DaemonSet (ignore_daemonsets skips it)",
            Self::LocalStorage => "uses emptyDir storage (delete_emptydir_data evicts it)",
            Self::Unmanaged => "is not managed by a controller (force evicts it)",
        }
    }
}

/// A pod the drain skips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedPod {
    /// The pod.
    pub pod: PodRef,
    /// Why it is skipped.
    pub reason: SkipReason,
}

/// A pod that blocks the drain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedPod {
    /// The pod.
    pub pod: PodRef,
    /// Why it blocks.
    pub reason: BlockReason,
}

/// What a drain would do, from the pods on the node and the options.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainPlan {
    /// Pods to evict, in listing order.
    pub evict: Vec<PodRef>,
    /// Pods left alone.
    pub skipped: Vec<SkippedPod>,
    /// Pods that stop the drain; when any, nothing is evicted.
    pub blocked: Vec<BlockedPod>,
}

/// One step of a running drain, in the order it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrainProgress {
    /// The plan: first, before anything changes.
    Planned {
        /// Pods that will be evicted.
        evict: Vec<PodRef>,
        /// Pods left alone.
        skipped: Vec<SkippedPod>,
    },
    /// The node is cordoned (`already` if it was before).
    Cordoned {
        /// Whether the node was unschedulable already.
        already: bool,
    },
    /// An eviction request is being sent.
    Evicting {
        /// The pod.
        pod: PodRef,
        /// 1 for the first request, then one more per retry.
        attempt: u32,
    },
    /// A PodDisruptionBudget (or the server's rate limiting) refused the eviction; it is
    /// retried after `retry_in`.
    Blocked {
        /// The pod.
        pod: PodRef,
        /// The attempt that was refused.
        attempt: u32,
        /// The server's explanation, one redacted line.
        reason: String,
        /// How long until the next attempt.
        retry_in: Duration,
    },
    /// The server accepted the eviction; the pod is terminating.
    Evicted {
        /// The pod.
        pod: PodRef,
    },
    /// The pod is gone from the node.
    Gone {
        /// The pod.
        pod: PodRef,
    },
    /// The pod could not be evicted or did not go in time.
    PodFailed {
        /// The pod.
        pod: PodRef,
        /// What went wrong, one redacted line.
        error: String,
    },
    /// The drain ended; the last event of a drain that did not fail as a whole.
    Finished(DrainSummary),
}

/// How a drain ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainSummary {
    /// The node.
    pub node: String,
    /// Pods that were evicted and are gone.
    pub evicted: Vec<PodRef>,
    /// Pods left alone.
    pub skipped: Vec<SkippedPod>,
    /// Pods that could not be evicted or did not go in time.
    pub failed: Vec<PodRef>,
    /// Whether this was a plan only: nothing was cordoned or evicted. `evicted` then holds
    /// the pods that would be.
    pub dry_run: bool,
}

impl DrainSummary {
    /// Whether every pod the plan named was evicted (always true for a dry run).
    pub fn is_complete(&self) -> bool {
        self.failed.is_empty()
    }
}
