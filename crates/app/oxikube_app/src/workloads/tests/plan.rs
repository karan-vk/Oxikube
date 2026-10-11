//! `parse_replicas`, `is_scalable` and the result line.

use oxikube_domain::ErrorKind;
use oxikube_domain::ids::Gvk;

use super::deployment;
use crate::workloads::{MAX_REPLICAS, ScaleResult, is_scalable, parse_replicas};

#[test]
fn replicas_are_whole_numbers_of_zero_or_more() {
    for (text, n) in [
        ("0", 0),
        ("1", 1),
        (" 12 ", 12),
        ("007", 7),
        ("2147483647", 2147483647),
    ] {
        assert_eq!(parse_replicas(text).unwrap(), n, "{text:?}");
    }
    assert_eq!(MAX_REPLICAS, 2147483647);
}

#[test]
fn a_field_that_is_not_a_count_says_why() {
    let message = |text: &str| {
        let error = parse_replicas(text).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation, "{text:?}");
        error.message().to_owned()
    };
    assert_eq!(message(""), "Enter the number of replicas.");
    assert_eq!(message("   "), "Enter the number of replicas.");
    assert_eq!(message("-1"), "Replicas cannot be negative.");
    assert_eq!(message(" -20 "), "Replicas cannot be negative.");
    for bad in ["two", "1.5", "3x", "+3", "-", "--1", "1e3", "٣"] {
        assert_eq!(
            message(bad),
            "Replicas must be a whole number, 0 or more.",
            "{bad:?}"
        );
    }
    assert_eq!(
        message("2147483648"),
        "Replicas cannot be more than 2147483647."
    );
    assert_eq!(
        message("99999999999999999999"),
        "Replicas cannot be more than 2147483647."
    );
}

#[test]
fn only_the_replica_controllers_scale() {
    let gvk = |g: &str, k: &str| Gvk::new(g, "v1", k);
    for (group, kind, scalable) in [
        ("apps", "Deployment", true),
        ("apps", "StatefulSet", true),
        ("apps", "ReplicaSet", true),
        ("", "ReplicationController", true),
        ("apps", "DaemonSet", false),
        ("batch", "Job", false),
        ("", "Pod", false),
        // A custom resource that borrows the name is not the built-in kind.
        ("example.io", "Deployment", false),
    ] {
        assert_eq!(is_scalable(&gvk(group, kind)), scalable, "{group}/{kind}");
    }
}

#[test]
fn the_result_reads_as_a_sentence() {
    let result = |before, after, dry_run| ScaleResult {
        target: deployment("a"),
        before,
        after,
        dry_run,
    };
    assert_eq!(
        result(Some(3), 5, false).message(),
        "Scaled Deployment shop/web from 3 to 5 replicas"
    );
    assert_eq!(
        result(Some(3), 1, false).message(),
        "Scaled Deployment shop/web from 3 to 1 replica"
    );
    assert_eq!(
        result(None, 0, false).message(),
        "Scaled Deployment shop/web to 0 replicas"
    );
    assert_eq!(
        result(Some(2), 4, true).message(),
        "Dry run: would scale Deployment shop/web from 2 to 4 replicas"
    );
}
