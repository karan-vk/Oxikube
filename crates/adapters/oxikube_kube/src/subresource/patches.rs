// Portions derived from kdash (https://github.com/kdash-rs/kdash), `src/network/mod.rs` at commit
// c303673 (v2.1.1): `ResourcePatch::to_merge_patch` (rollout-restart annotation, cordon and
// uncordon, cronjob suspend, scale replicas). MIT licence; the full text follows. Modifications (c)
// Oxikube contributors: the restart timestamp is an argument instead of read from the clock, the
// bodies are returned with their patch kind as an `oxikube_ports::Patch`, and the cases are
// pinned by exact-JSON tests.
//
// Copyright (c) 2021 Deepu K Sasidharan
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! Patch builders: the small, exact merge patches behind scale, rollout restart, cordon and
//! cronjob suspend.
//!
//! Pure functions of their inputs. Each [`ResourcePatch`] becomes a JSON merge patch
//! ([`to_merge_patch`](ResourcePatch::to_merge_patch)) and the matching
//! [`Patch`] ([`to_patch`](ResourcePatch::to_patch)), which goes to `ResourceWriter::patch`
//! (or, for [`Scale`](ResourcePatch::Scale), to `patch_subresource` on `scale`). They hold no
//! kube types, so they could move to the domain if another adapter needs them.

use jiff::Timestamp;
use oxikube_ports::Patch;
use serde_json::{Value, json};

/// The pod template annotation `kubectl rollout restart` sets; a changed value changes the
/// template, which makes the workload controller roll new pods.
pub const RESTARTED_AT_ANNOTATION: &str = "kubectl.kubernetes.io/restartedAt";

/// A small, well-known change to one object, expressed as a merge patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourcePatch {
    /// `spec.replicas = n`. The body of a `scale` subresource patch; also valid on a
    /// Deployment, StatefulSet or ReplicaSet directly.
    Scale(i32),
    /// Rolling restart of a Deployment, StatefulSet or DaemonSet: stamps the pod template with
    /// [`RESTARTED_AT_ANNOTATION`] set to `at` (RFC 3339, UTC, whole seconds, as kubectl does).
    RolloutRestart {
        /// The restart time, from the app's clock.
        at: Timestamp,
    },
    /// Marks a Node unschedulable (`spec.unschedulable = true`). Existing pods stay; draining
    /// is a separate operation (E04-S07).
    Cordon,
    /// Marks a Node schedulable again (`spec.unschedulable = false`).
    Uncordon,
    /// Suspends (`true`) or resumes (`false`) a CronJob (`spec.suspend`).
    CronJobSuspend(bool),
}

impl ResourcePatch {
    /// The JSON merge patch (RFC 7386) body.
    pub fn to_merge_patch(&self) -> Value {
        match self {
            Self::Scale(replicas) => json!({"spec": {"replicas": replicas}}),
            Self::RolloutRestart { at } => json!({
                "spec": {"template": {"metadata": {"annotations": {
                    RESTARTED_AT_ANNOTATION: at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string(),
                }}}}
            }),
            Self::Cordon => json!({"spec": {"unschedulable": true}}),
            Self::Uncordon => json!({"spec": {"unschedulable": false}}),
            Self::CronJobSuspend(suspend) => json!({"spec": {"suspend": suspend}}),
        }
    }

    /// The patch to send: [`to_merge_patch`](Self::to_merge_patch) with the merge patch kind.
    pub fn to_patch(&self) -> Patch {
        Patch::merge(self.to_merge_patch())
    }
}
