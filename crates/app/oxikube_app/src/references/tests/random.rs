//! Property test: any sequence of Pod deltas leaves the index equal to one built from scratch
//! from the Pods that remain.

use std::collections::BTreeMap;

use oxikube_domain::Resource;
use proptest::prelude::*;
use serde_json::json;

use super::{object, ops, owned_by, pod_in, ready};
use crate::references::ReferenceIndex;
use crate::store::RowOp;

#[derive(Debug, Clone)]
enum Step {
    Upsert { pod: u8, variant: u8 },
    Delete { pod: u8 },
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (0u8..12, 0u8..9).prop_map(|(pod, variant)| Step::Upsert { pod, variant }),
        1 => (0u8..12).prop_map(|pod| Step::Delete { pod }),
    ]
}

/// The Pod `pod` (half in `shop`, half in `lab`) in one of nine shapes that overlap in what they
/// name, so objects gain and lose users.
fn make(pod: u8, variant: u8) -> ((String, String), Resource) {
    let namespace = if pod.is_multiple_of(2) { "shop" } else { "lab" };
    let name = format!("pod-{pod:02}");
    let cm = |n: &str| json!({"name": "v", "configMap": {"name": n}});
    let spec = match variant {
        0 => json!({}),
        1 | 5 | 6 | 7 => json!({"volumes": [cm("a")]}),
        2 => json!({"volumes": [cm("a"), {"name": "s", "secret": {"secretName": "s"}}]}),
        3 => json!({"serviceAccountName": "sa", "volumes": [
            {"name": "d", "persistentVolumeClaim": {"claimName": "c"}}]}),
        4 => json!({"priorityClassName": "p", "volumes": [cm("b")]}),
        _ => json!({"imagePullSecrets": [{"name": "s"}], "volumes": [cm("a")]}),
    };
    let phase = if variant == 5 { "Succeeded" } else { "Running" };
    let resource = pod_in(&name, namespace, spec, phase);
    let resource = match variant {
        6 => owned_by(resource, "apps/v1", "ReplicaSet", "rs1"),
        7 => owned_by(resource, "apps/v1", "ReplicaSet", "rs2"),
        _ => resource,
    };
    ((namespace.to_owned(), name), resource)
}

fn rank(model: &BTreeMap<(String, String), Resource>, key: &(String, String)) -> usize {
    model.range(..key.clone()).count()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn random_deltas_equal_a_rebuild(
        batches in proptest::collection::vec(proptest::collection::vec(step(), 1..4), 1..40)
    ) {
        let index = ReferenceIndex::new();
        index.apply(&super::snapshot(vec![]));
        let mut model: BTreeMap<(String, String), Resource> = BTreeMap::new();
        for batch in batches {
            let mut row_ops = Vec::new();
            for step in batch {
                match step {
                    Step::Upsert { pod, variant } => {
                        let (key, resource) = make(pod, variant);
                        let at = rank(&model, &key);
                        let known = model.insert(key, resource.clone()).is_some();
                        row_ops.push(if known {
                            RowOp::Update { index: at, object: object(resource) }
                        } else {
                            RowOp::Insert { index: at, object: object(resource) }
                        });
                    }
                    Step::Delete { pod } => {
                        let (key, _) = make(pod, 0);
                        if model.contains_key(&key) {
                            row_ops.push(RowOp::Remove { index: rank(&model, &key) });
                            model.remove(&key);
                        }
                    }
                }
            }
            index.apply(&ops(row_ops));
            let fresh = ready(model.values().cloned().collect());
            prop_assert_eq!(index.dump(), fresh.dump());
            prop_assert_eq!(index.pods(), fresh.pods());
            prop_assert_eq!(index.indexed(), fresh.indexed());
        }
    }
}
