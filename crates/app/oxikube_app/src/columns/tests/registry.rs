//! Invariants of the catalogue and of `columns()`: ids, wide ordering, feed policy, metrics
//! gating, memoisation, metadata-only objects.

use std::collections::HashSet;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::ids::Gvk;
use oxikube_domain::{Capabilities, Resource};
use oxikube_testkit::fixtures as fx;

use super::{cell, check, now};
use crate::columns::{Cell, ColumnId, ColumnProvider, CoreColumns, Metric, MetricsSource, Tone};
use crate::store::{FeedKind, FeedPolicy, StoreObject, TableObject};

fn gvk(group: &str, kind: &str) -> Gvk {
    Gvk::new(group, "v1", kind)
}

#[test]
fn the_catalogue_covers_about_forty_kinds() {
    let kinds: Vec<_> = CoreColumns::new().kinds().collect();
    assert!(kinds.len() >= 38, "{} kinds", kinds.len());
    let unique: HashSet<_> = kinds.iter().collect();
    assert_eq!(unique.len(), kinds.len(), "a kind is listed twice");
}

#[test]
fn every_kind_has_unique_stable_looking_ids_and_an_age_or_name() {
    let provider = CoreColumns::new();
    for (group, kind) in provider.kinds() {
        let columns = provider.columns(&gvk(group, kind), Capabilities::all());
        let ids: Vec<&str> = columns.iter().map(|c| c.id.as_str()).collect();
        let unique: HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "{kind}: duplicate ids {ids:?}");
        for id in &ids {
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{kind}: id `{id}` is not kebab-case"
            );
        }
        assert!(ids.contains(&"name"), "{kind}: no name column");
        assert!(
            columns
                .iter()
                .any(|c| c.is_default() && c.id == *ColumnId::NAME)
                || columns.iter().any(|c| c.id == *ColumnId::NAME),
            "{kind}"
        );
        assert!(ids.contains(&"age"), "{kind}: no age column");
    }
}

#[test]
fn default_columns_come_before_wide_ones() {
    let provider = CoreColumns::new();
    for (group, kind) in provider.kinds() {
        let columns = provider.columns(&gvk(group, kind), Capabilities::all());
        let first_wide = columns.iter().position(|c| c.wide);
        if let Some(i) = first_wide {
            assert!(
                columns[i..].iter().all(|c| c.wide),
                "{kind}: a default column follows a wide one"
            );
        }
        assert!(
            columns.iter().any(|c| !c.wide),
            "{kind}: nothing visible by default"
        );
    }
}

#[test]
fn wide_flags_follow_kubectl_wide() {
    let columns = CoreColumns::new().columns(&gvk("", "Pod"), Capabilities::empty());
    let wide = |id: &str| columns.iter().find(|c| c.id == *id).unwrap().wide;
    for id in [
        "name",
        "namespace",
        "ready",
        "status",
        "restarts",
        "node",
        "ip",
        "age",
    ] {
        assert!(!wide(id), "{id} is a default column");
    }
    for id in ["qos", "controlled-by", "nominated-node", "labels"] {
        assert!(wide(id), "{id} is a wide column");
    }
}

#[test]
fn pods_default_to_the_columns_that_matter_first_and_node_and_ip_last() {
    let ids = |provider: &CoreColumns, caps| -> Vec<String> {
        provider
            .columns(&gvk("", "Pod"), caps)
            .iter()
            .filter(|c| c.is_default())
            .map(|c| c.id.to_string())
            .collect()
    };
    // Namespace sits after the name for an all-namespaces list; the table drops it for one.
    let plain = CoreColumns::new();
    assert_eq!(
        ids(&plain, Capabilities::METRICS),
        [
            "name",
            "namespace",
            "status",
            "ready",
            "restarts",
            "age",
            "node",
            "ip"
        ]
    );
    let metered = CoreColumns::new().with_metrics(Arc::new(FixedMetrics));
    assert_eq!(
        ids(&metered, Capabilities::METRICS),
        [
            "name",
            "namespace",
            "status",
            "ready",
            "restarts",
            "cpu",
            "memory",
            "age",
            "node",
            "ip"
        ]
    );
}

#[test]
fn the_catalogue_and_the_feed_policy_agree() {
    let provider = CoreColumns::new();
    let policy = FeedPolicy::new();
    for (group, kind) in provider.kinds() {
        assert_ne!(
            policy.plan(&gvk(group, kind)).kind,
            FeedKind::Table,
            "{kind} has core columns but is read through the Table feed"
        );
    }
    for (group, kind) in crate::store::core_kinds() {
        assert!(
            provider.knows(&gvk(group, kind)),
            "the policy serves {kind} but it has no core columns"
        );
    }
}

#[test]
fn metrics_columns_need_the_metrics_capability() {
    let provider = CoreColumns::new().with_metrics(Arc::new(FixedMetrics));
    let ids = |caps| -> Vec<String> {
        provider
            .columns(&gvk("", "Pod"), caps)
            .iter()
            .map(|c| c.id.to_string())
            .collect()
    };
    assert!(!ids(Capabilities::empty()).contains(&"cpu".to_owned()));
    assert!(!ids(Capabilities::MUTATE | Capabilities::LOGS).contains(&"memory".to_owned()));
    let with = ids(Capabilities::METRICS);
    assert!(with.contains(&"cpu".to_owned()) && with.contains(&"memory".to_owned()));
    // The same hook on Nodes.
    assert!(
        provider
            .columns(&gvk("", "Node"), Capabilities::METRICS)
            .iter()
            .any(|c| c.id == *"cpu")
    );
}

#[test]
fn metrics_columns_are_absent_without_a_registered_source_even_with_the_capability() {
    let provider = CoreColumns::new();
    for kind in ["Pod", "Node"] {
        let ids: Vec<String> = provider
            .columns(&gvk("", kind), Capabilities::METRICS)
            .iter()
            .map(|c| c.id.to_string())
            .collect();
        assert!(
            !ids.contains(&"cpu".to_owned()) && !ids.contains(&"memory".to_owned()),
            "{kind}: {ids:?}"
        );
    }
}

#[test]
fn columns_are_memoised_per_kind_and_capabilities_and_ignore_the_version() {
    let provider = CoreColumns::new();
    let a = provider.columns(&gvk("", "Pod"), Capabilities::METRICS);
    let b = provider.columns(&Gvk::new("", "v1beta1", "Pod"), Capabilities::METRICS);
    assert!(
        Arc::ptr_eq(&a, &b),
        "same (kind, caps) must share one allocation"
    );
    let c = provider.columns(&gvk("", "Pod"), Capabilities::empty());
    assert!(!Arc::ptr_eq(&a, &c));
    let hpa_v1 = provider.columns(
        &Gvk::new("autoscaling", "v1", "HorizontalPodAutoscaler"),
        Capabilities::empty(),
    );
    let hpa_v2 = provider.columns(
        &Gvk::new("autoscaling", "v2", "HorizontalPodAutoscaler"),
        Capabilities::empty(),
    );
    assert!(Arc::ptr_eq(&hpa_v1, &hpa_v2));
}

#[test]
fn unknown_kinds_get_the_generic_set() {
    let provider = CoreColumns::new();
    let kind = Gvk::new("example.com", "v1", "Widget");
    assert!(!provider.knows(&kind));
    let ids: Vec<String> = provider
        .columns(&kind, Capabilities::all())
        .iter()
        .map(|c| c.id.to_string())
        .collect();
    assert_eq!(ids, ["name", "namespace", "age", "labels"]);
}

struct FixedMetrics;

impl MetricsSource for FixedMetrics {
    fn cell(&self, object: &Resource, metric: Metric, _now: Timestamp) -> Option<Cell<'static>> {
        if &*object.meta.name != "web-running" {
            return None;
        }
        Some(match metric {
            Metric::Cpu => Cell::quantity("250m", "250m".parse().unwrap()),
            Metric::Memory => Cell::quantity("128Mi", "128Mi".parse().unwrap()),
        })
    }
}

#[test]
fn a_registered_metrics_source_fills_the_hooks_and_missing_samples_stay_pending() {
    let provider = CoreColumns::new().with_metrics(Arc::new(FixedMetrics));
    let sampled = fx::pod_running();
    let unsampled = fx::pod_pending();
    let cpu = ColumnId::new("cpu");
    let memory = ColumnId::new("memory");
    assert_eq!(
        provider.resource_cell(&sampled, &cpu, now()).display(),
        "250m"
    );
    assert_eq!(
        provider.resource_cell(&sampled, &memory, now()).display(),
        "128Mi"
    );
    let missing = provider.resource_cell(&unsampled, &cpu, now());
    assert!(missing.is_pending(), "no sample: show nothing, not 0");
    assert_eq!(missing.display(), "");
}

#[test]
fn metadata_only_objects_fill_only_metadata_columns() {
    let partial = fx::pod_running().into_partial();
    check(
        &partial,
        &[
            ("name", "web-running"),
            ("namespace", "demo"),
            ("age", "27h"),
            ("ready", ""),
            ("status", ""),
            ("node", ""),
        ],
    );
    assert!(cell(&partial, "ready").is_blank());
}

#[test]
fn table_rows_answer_the_metadata_columns_through_the_core_provider() {
    let res = fx::service();
    let row = StoreObject::Row(TableObject {
        meta: res.meta,
        cells: vec![],
        object: None,
    });
    let provider = CoreColumns::new();
    let at = |id: &str| {
        provider
            .cell(&row, &ColumnId::new(id), now())
            .display()
            .to_owned()
    };
    assert_eq!(at("name"), "web");
    assert_eq!(at("namespace"), "demo");
    assert_eq!(at("age"), "27h");
    assert_eq!(at("ports"), "");
}

#[test]
fn pending_and_blank_cells_have_no_tone() {
    assert_eq!(Cell::Pending.tone(), Tone::Neutral);
    assert!(Cell::Pending.is_pending());
    assert!(Cell::empty().is_blank());
    assert!(!Cell::int(0).is_blank(), "a real zero is not blank");
}
