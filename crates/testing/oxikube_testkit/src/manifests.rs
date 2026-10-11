//! Large YAML manifests for the manifest editor's performance work (E10-S11): what
//! `kubectl get pods -A -o yaml` prints for a big cluster, generated to a size, with a schema
//! error planted every so many pods.
//!
//! The pods use only the fields the trimmed Pod schema of `fixtures::openapi` knows
//! ([`pod_schema`]), so the only findings are the planted ones (`containerPort: "8080"`, a
//! string where an integer belongs) and a test can count them. `status` is printed as a real
//! dump has it; the validator skips it at an object's root.
//!
//! The generator is deterministic (no randomness, no clock), so a perf run and a test see the
//! same bytes.

use std::fmt::Write as _;
use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::{JsonSchema, root_schema_for};

use crate::fixtures::openapi;

/// 5 MiB: the epic's "5 MB `kubectl get -o yaml` dump".
pub const FIVE_MB: usize = 5 * 1024 * 1024;

/// A `kind: List` of pods of at least `min_bytes` (it stops after the pod that crosses it). Every
/// `faulty_every`-th pod (the last of each run of that many, counting from 1) declares its port
/// as the string `"8080"`, one `type-mismatch` per faulty pod; `None` plants none.
pub fn pod_list_yaml(min_bytes: usize, faulty_every: Option<usize>) -> String {
    let mut out = String::with_capacity(min_bytes + 1024);
    out.push_str("apiVersion: v1\nkind: List\nitems:\n");
    let mut i = 0usize;
    while out.len() < min_bytes {
        let faulty = faulty_every.is_some_and(|n| n > 0 && i % n == n - 1);
        let port = if faulty { "\"8080\"" } else { "8080" };
        let n = i % 50;
        let _ = write!(
            out,
            "- apiVersion: v1
  kind: Pod
  metadata:
    name: web-{i:06}
    namespace: team-{n:02}
  spec:
    restartPolicy: Always
    containers:
    - name: app
      image: registry.example.com/team-{n:02}/app:1.{n}
      ports:
      - containerPort: {port}
        protocol: TCP
    - name: sidecar
      image: registry.example.com/proxy:2.4
  status:
    phase: Running
    podIP: 10.{a}.{b}.{c}
    conditions:
    - type: Ready
      status: \"True\"
",
            a = i / 65_536 % 256,
            b = i / 256 % 256,
            c = i % 256,
        );
        i += 1;
    }
    out.push_str("metadata:\n  resourceVersion: \"\"\n");
    out
}

/// How many pods [`pod_list_yaml`] wrote into `text`.
pub fn pod_count(text: &str) -> usize {
    text.matches("\n- apiVersion: v1\n  kind: Pod\n").count()
}

/// How many planted errors `text` (from [`pod_list_yaml`]) holds.
pub fn faulty_count(text: &str) -> usize {
    text.matches("containerPort: \"8080\"").count()
}

/// The kind of every object in [`pod_list_yaml`].
pub fn pod_gvk() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// The Pod schema the dumps are checked against: the trimmed core `v1` OpenAPI fixture,
/// flattened for `v1/Pod`.
///
/// # Panics
///
/// When the fixture no longer defines a Pod (a testkit bug).
pub fn pod_schema() -> Arc<JsonSchema> {
    Arc::new(root_schema_for(&openapi::pod_core_v1(), &pod_gvk()).expect("the fixture has a Pod"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dumps_reach_their_size_and_plant_their_errors() {
        let text = pod_list_yaml(64 * 1024, Some(10));
        assert!(text.len() >= 64 * 1024);
        assert!(text.len() < 64 * 1024 + 1024, "stops right after the size");
        let pods = pod_count(&text);
        assert!(pods > 100, "{pods}");
        assert_eq!(faulty_count(&text), pods / 10);
        assert_eq!(faulty_count(&pod_list_yaml(8 * 1024, None)), 0);
        assert_eq!(pod_list_yaml(4096, Some(3)), pod_list_yaml(4096, Some(3)));
    }

    #[test]
    fn the_pod_schema_is_flattened() {
        let schema = pod_schema();
        assert!(schema.properties.contains_key("spec"));
    }
}
