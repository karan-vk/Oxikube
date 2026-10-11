//! Property tests: order independence, spelling invariance, percent bounds and monotonicity.

use proptest::prelude::*;
use serde_json::{Value, json};

use super::oracle::{Ref, model};
use super::{container, pod, q};
use crate::quantity::{Quantity, QuantityFormat};
use crate::utilisation::{GpuVendors, PodResources, aggregate_pod_requests_limits, percent};

/// One container's amounts: millicores and MiB, `0` meaning unset.
#[derive(Debug, Clone, Copy)]
struct Spec {
    cpu_req_m: u32,
    cpu_lim_m: u32,
    mem_req_mi: u32,
    mem_lim_mi: u32,
}

fn spec() -> impl Strategy<Value = Spec> {
    let cpu = prop_oneof![Just(0u32), 1u32..8000];
    let mem = prop_oneof![Just(0u32), 1u32..16384];
    (cpu.clone(), cpu, mem.clone(), mem).prop_map(|(a, b, c, d)| Spec {
        cpu_req_m: a,
        cpu_lim_m: b,
        mem_req_mi: c,
        mem_lim_mi: d,
    })
}

/// The container, with the amounts spelled one of several equivalent ways.
fn render(name: &str, s: Spec, spelling: u8) -> Value {
    let cpu = |m: u32| match spelling % 3 {
        0 => format!("{m}m"),
        1 => format!("{}u", u64::from(m) * 1000),
        _ => format!("{}", f64::from(m) / 1000.0),
    };
    let mem = |mi: u32| match spelling % 3 {
        0 => format!("{mi}Mi"),
        1 => format!("{}Ki", u64::from(mi) * 1024),
        _ => format!("{}", u64::from(mi) * 1024 * 1024),
    };
    let map = |c: u32, m: u32, cpu_f: &dyn Fn(u32) -> String, mem_f: &dyn Fn(u32) -> String| {
        let mut out = serde_json::Map::new();
        if c > 0 {
            out.insert("cpu".into(), json!(cpu_f(c)));
        }
        if m > 0 {
            out.insert("memory".into(), json!(mem_f(m)));
        }
        if out.is_empty() {
            Value::Null
        } else {
            Value::Object(out)
        }
    };
    container(
        name,
        map(s.cpu_req_m, s.mem_req_mi, &cpu, &mem),
        map(s.cpu_lim_m, s.mem_lim_mi, &cpu, &mem),
    )
}

fn reference(s: Spec) -> Ref {
    Ref {
        cpu_req: i128::from(s.cpu_req_m) * 1_000_000,
        cpu_lim: i128::from(s.cpu_lim_m) * 1_000_000,
        mem_req: i128::from(s.mem_req_mi) * 1024 * 1024,
        mem_lim: i128::from(s.mem_lim_mi) * 1024 * 1024,
    }
}

fn aggregate(containers: &[Spec], inits: &[Spec], spelling: u8) -> PodResources {
    let render_all = |specs: &[Spec], prefix: &str| -> Vec<Value> {
        specs
            .iter()
            .enumerate()
            .map(|(i, s)| render(&format!("{prefix}{i}"), *s, spelling))
            .collect()
    };
    let p = pod(
        "p",
        "ns",
        None,
        "Running",
        json!({"containers": render_all(containers, "c"), "initContainers": render_all(inits, "i")}),
    );
    aggregate_pod_requests_limits(&p, &GpuVendors::default()).unwrap()
}

proptest! {
    /// The implementation equals the integer model of the tool's semantics.
    #[test]
    fn matches_the_oracle(
        containers in prop::collection::vec(spec(), 0..6),
        inits in prop::collection::vec(spec(), 0..3),
        spelling in 0u8..3,
    ) {
        let got = aggregate(&containers, &inits, spelling);
        let c: Vec<Ref> = containers.iter().copied().map(reference).collect();
        let i: Vec<Ref> = inits.iter().copied().map(reference).collect();
        let want = model(&c, &i);
        prop_assert_eq!(got.requests.cpu, Quantity::from_nanos(want.cpu_req));
        prop_assert_eq!(got.limits.cpu, Quantity::from_nanos(want.cpu_lim));
        prop_assert_eq!(got.requests.memory, Quantity::from_nanos(want.mem_req * 1_000_000_000));
        prop_assert_eq!(got.limits.memory, Quantity::from_nanos(want.mem_lim * 1_000_000_000));
    }

    /// Container and init-container order never changes the pod's figures.
    #[test]
    fn aggregation_is_order_independent(
        mut containers in prop::collection::vec(spec(), 0..6),
        mut inits in prop::collection::vec(spec(), 0..4),
        k in 0usize..6,
    ) {
        let before = aggregate(&containers, &inits, 0);
        if !containers.is_empty() { let n = k % containers.len(); containers.rotate_left(n); }
        inits.reverse();
        prop_assert_eq!(aggregate(&containers, &inits, 0), before);
    }

    /// How a quantity is spelled (`500m`, `500000u`, `0.5`; `Mi`, `Ki`, bytes) changes nothing.
    #[test]
    fn spelling_does_not_change_the_result(
        containers in prop::collection::vec(spec(), 1..5),
        inits in prop::collection::vec(spec(), 0..3),
    ) {
        let a = aggregate(&containers, &inits, 0);
        prop_assert_eq!(aggregate(&containers, &inits, 1), a);
        prop_assert_eq!(aggregate(&containers, &inits, 2), aggregate(&containers, &inits, 0));
    }

    /// `percent` is non-negative, `None` on a zero base, and never decreases with usage.
    #[test]
    fn percent_bounds_and_monotonicity(a in 0u64..10_000_000, b in 0u64..10_000_000, base in 0u64..10_000_000) {
        let (lo, hi) = (a.min(b), a.max(b));
        let milli = |n: u64| Quantity::from_milli(n as i64);
        let base_q = milli(base);
        match (percent(&milli(lo), &base_q), percent(&milli(hi), &base_q)) {
            (None, None) => prop_assert_eq!(base, 0),
            (Some(p_lo), Some(p_hi)) => {
                prop_assert!(base > 0);
                prop_assert!(p_lo >= 0.0 && p_lo <= p_hi);
                if hi <= base { prop_assert!(p_hi <= 100.0); }
                // Agrees with the E02 float implementation to display precision.
                let float = milli(hi).percent_of(&base_q).unwrap();
                prop_assert!((p_hi - float).abs() <= float.abs() * 1e-9 + 1e-9);
            }
            other => prop_assert!(false, "inconsistent: {other:?}"),
        }
    }

    /// Scaling usage and base by one unit suffix (`m` to `u` to `n`, `Ki` to `Mi`) keeps the percent.
    #[test]
    fn unit_scaling_keeps_the_percent(used in 0u64..1_000_000, base in 1u64..1_000_000) {
        let milli = percent(&q(&format!("{used}m")), &q(&format!("{base}m")));
        let micro = percent(&q(&format!("{}u", used * 1000)), &q(&format!("{}u", base * 1000)));
        let nano = percent(&q(&format!("{}n", used * 1_000_000)), &q(&format!("{}n", base * 1_000_000)));
        prop_assert_eq!(milli, micro);
        prop_assert_eq!(milli, nano);
        let ki = percent(&q(&format!("{used}Ki")), &q(&format!("{base}Ki")));
        let bytes = percent(&q(&format!("{}", used * 1024)), &q(&format!("{}", base * 1024)));
        prop_assert_eq!(ki, bytes);
        let binary = q(&format!("{used}Ki")).with_format(QuantityFormat::BinarySI);
        prop_assert_eq!(percent(&binary, &q(&format!("{base}Ki"))), ki);
    }
}
