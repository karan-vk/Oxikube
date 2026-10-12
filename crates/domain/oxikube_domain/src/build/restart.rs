// Portions derived from kdash (https://github.com/kdash-rs/kdash), `src/network/mod.rs` at commit
// c303673 (v2.1.1): `ResourcePatch::to_merge_patch`, the rollout-restart case (the pod template
// annotation `kubectl.kubernetes.io/restartedAt`). MIT licence; the full text follows.
// Modifications (c) Oxikube contributors: the timestamp is an argument (the app's `ClockPort`
// supplies it, not the system clock), the builder lives in the domain so the app's
// `workload::Restart` handler and the kube adapter's `ResourcePatch` build the same body, and the
// output is pinned by exact-JSON tests.
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

//! `restart_patch`: the merge patch behind `kubectl rollout restart`.

use jiff::Timestamp;
use serde_json::{Value, json};

/// The pod template annotation `kubectl rollout restart` sets; a changed value changes the
/// template, which makes the workload controller roll new pods.
pub const RESTARTED_AT_ANNOTATION: &str = "kubectl.kubernetes.io/restartedAt";

/// The JSON merge patch (RFC 7386) that restarts a Deployment, StatefulSet or DaemonSet: it stamps
/// `spec.template.metadata.annotations` with [`RESTARTED_AT_ANNOTATION`] set to `at` (RFC 3339,
/// UTC, whole seconds, as kubectl writes it).
///
/// A merge patch creates the `annotations` map when the template has none, and leaves the
/// template's other annotations alone. Pure: `at` comes from the caller's clock.
pub fn restart_patch(at: Timestamp) -> Value {
    json!({
        "spec": {"template": {"metadata": {"annotations": {
            RESTARTED_AT_ANNOTATION: restart_stamp(at),
        }}}}
    })
}

/// `at` as `kubectl rollout restart` writes it: RFC 3339, UTC, whole seconds
/// (`2026-10-06T12:34:56Z`).
pub fn restart_stamp(at: Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use serde_json::json;

    use super::{RESTARTED_AT_ANNOTATION, restart_patch, restart_stamp};

    #[test]
    fn the_patch_stamps_only_the_template_annotation() {
        let at: Timestamp = "2026-10-06T12:34:56.789Z".parse().unwrap();
        assert_eq!(
            restart_patch(at),
            json!({"spec": {"template": {"metadata": {"annotations": {
                "kubectl.kubernetes.io/restartedAt": "2026-10-06T12:34:56Z"
            }}}}})
        );
        assert_eq!(restart_stamp(at), "2026-10-06T12:34:56Z");
        assert_eq!(RESTARTED_AT_ANNOTATION, "kubectl.kubernetes.io/restartedAt");
    }
}
