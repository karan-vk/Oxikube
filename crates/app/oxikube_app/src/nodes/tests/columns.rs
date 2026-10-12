//! The node table's columns and cells.

use std::collections::HashMap;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::Capabilities;
use oxikube_domain::ids::Gvk;
use oxikube_testkit::node;

use crate::columns::{CellSort, Column, ColumnId, ColumnProvider, Tone};
use crate::nodes::{NodeColumns, NodeUsage, NodeUsageSource};
use crate::store::StoreObject;
use crate::{KindColumns, KindMeta};

fn gvk() -> Gvk {
    Gvk::new("", "v1", "Node")
}

fn now() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

fn columns(provider: &dyn ColumnProvider) -> Arc<[Column]> {
    provider.columns(&gvk(), Capabilities::default())
}

fn object(builder: oxikube_testkit::NodeBuilder) -> Arc<StoreObject> {
    Arc::new(StoreObject::Resource(builder.build()))
}

fn text(provider: &dyn ColumnProvider, object: &StoreObject, column: &str) -> String {
    provider
        .cell(object, &ColumnId::new(column), now())
        .display()
        .to_owned()
}

struct Usage(HashMap<&'static str, NodeUsage>);

impl NodeUsageSource for Usage {
    fn usage(&self, node: &str) -> Option<NodeUsage> {
        self.0.get(node).copied()
    }
}

#[test]
fn the_default_columns_are_the_stories_and_the_rest_are_wide() {
    let provider = NodeColumns::new();
    let all = columns(&provider);
    let shown: Vec<&str> = all
        .iter()
        .filter(|c| !c.wide)
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(
        shown,
        [
            "name",
            "status",
            "roles",
            "version",
            "os-image",
            "kernel",
            "internal-ip",
            "pods",
            "cpu",
            "cpu-percent",
            "memory",
            "memory-percent",
            "taints",
            "schedulable",
            "age"
        ]
    );
    let titles: Vec<&str> = all.iter().filter(|c| !c.wide).map(|c| &*c.title).collect();
    assert_eq!(
        titles,
        [
            "Name",
            "Status",
            "Roles",
            "Version",
            "OS Image",
            "Kernel",
            "Internal IP",
            "Pods",
            "CPU",
            "CPU%",
            "Mem",
            "Mem%",
            "Taints",
            "Schedulable",
            "Age"
        ]
    );
    let wide: Vec<&str> = all
        .iter()
        .filter(|c| c.wide)
        .map(|c| c.id.as_str())
        .collect();
    assert!(wide.contains(&"instance-type") && wide.contains(&"zone"));
    assert!(wide.contains(&"external-ip") && wide.contains(&"labels"));
    // Every column id is unique, and the numeric ones sort as numbers.
    let mut ids: Vec<_> = all.iter().map(|c| c.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), all.len());
    let pods = all.iter().find(|c| c.id == "pods").unwrap();
    assert_eq!(pods.sort, crate::columns::SortKind::Number);
    assert!(
        !pods.sortable,
        "a lookup in the Pod feed cannot be ranked under the node feed's lock"
    );
    assert!(all.iter().filter(|c| c.id != "pods").all(|c| c.sortable));
}

#[test]
fn the_catalogue_cells_come_through() {
    let provider = NodeColumns::new();
    let n = object(
        node()
            .name("cp-1")
            .role("control-plane")
            .kubelet("v1.33.4")
            .taint("node-role.kubernetes.io/control-plane", "", "NoSchedule"),
    );
    assert_eq!(text(&provider, &n, "name"), "cp-1");
    assert_eq!(text(&provider, &n, "status"), "Ready");
    assert_eq!(text(&provider, &n, "roles"), "control-plane");
    assert_eq!(text(&provider, &n, "version"), "v1.33.4");
    assert_eq!(
        text(&provider, &n, "os-image"),
        "Debian GNU/Linux 12 (bookworm)"
    );
    assert_eq!(text(&provider, &n, "kernel"), "6.8.0");
    assert_eq!(text(&provider, &n, "internal-ip"), "172.18.0.3");
    assert_eq!(text(&provider, &n, "taints"), "1");
    assert_eq!(text(&provider, &n, "schedulable"), "Yes");
}

#[test]
fn a_cordoned_node_shows_scheduling_disabled_and_is_not_schedulable() {
    let provider = NodeColumns::new();
    let n = object(node().cordoned());
    assert_eq!(text(&provider, &n, "status"), "Ready,SchedulingDisabled");
    let status = provider.cell(&n, &ColumnId::new("status"), now());
    assert_eq!(status.tone(), Tone::Warn, "the badge is amber");
    let schedulable = provider.cell(&n, &ColumnId::new("schedulable"), now());
    assert_eq!(schedulable.display(), "No");
    assert_eq!(schedulable.tone(), Tone::Warn);

    let not_ready = object(node().not_ready());
    assert_eq!(
        provider
            .cell(&not_ready, &ColumnId::new("status"), now())
            .tone(),
        Tone::Error
    );
}

#[test]
fn instance_type_and_zone_come_from_the_labels() {
    let provider = NodeColumns::new();
    let n = object(
        node()
            .label("node.kubernetes.io/instance-type", "m5.large")
            .label("topology.kubernetes.io/zone", "eu-west-1a"),
    );
    assert_eq!(text(&provider, &n, "instance-type"), "m5.large");
    assert_eq!(text(&provider, &n, "zone"), "eu-west-1a");
    let bare = object(node());
    assert_eq!(text(&provider, &bare, "instance-type"), "");
}

#[test]
fn pods_and_metrics_show_a_dash_until_something_supplies_them() {
    let provider = NodeColumns::new();
    let n = object(node());
    for column in ["pods", "cpu", "memory", "cpu-percent", "memory-percent"] {
        let cell = provider.cell(&n, &ColumnId::new(column), now());
        assert_eq!(cell.display(), "–", "{column}");
        assert_eq!(cell.sort(), CellSort::None, "{column} sinks when sorted");
    }
}

#[test]
fn cpu_and_memory_percent_are_usage_over_allocatable() {
    let usage = Usage(HashMap::from([
        (
            "worker-1",
            NodeUsage {
                cpu_millis: 1000,
                memory_bytes: 12 << 30,
            },
        ),
        (
            "hot",
            NodeUsage {
                cpu_millis: 3800,
                memory_bytes: 1 << 30,
            },
        ),
    ]));
    let provider = NodeColumns::new().with_usage(Arc::new(usage));
    // The builder's default node: 4 CPU, 16Gi allocatable.
    let n = object(node());
    assert_eq!(text(&provider, &n, "cpu-percent"), "25%");
    assert_eq!(text(&provider, &n, "memory-percent"), "75%");
    // The absolute usage beside the percent, sorted by the exact quantity.
    let cpu = provider.cell(&n, &ColumnId::new("cpu"), now());
    assert_eq!(cpu.display(), "1000m");
    assert_eq!(
        cpu.sort(),
        CellSort::Quantity(oxikube_domain::Quantity::from_milli(1000))
    );
    let memory = provider.cell(&n, &ColumnId::new("memory"), now());
    assert_eq!(memory.display(), "12288Mi");
    assert_eq!(
        memory.sort(),
        CellSort::Quantity(oxikube_domain::Quantity::from_value(12 << 30))
    );
    assert_eq!(
        provider
            .cell(&n, &ColumnId::new("memory-percent"), now())
            .tone(),
        Tone::Warn
    );
    let hot = object(node().name("hot"));
    let cell = provider.cell(&hot, &ColumnId::new("cpu-percent"), now());
    assert_eq!(cell.display(), "95%");
    assert_eq!(cell.tone(), Tone::Error);
    // A node without a sample.
    let cold = object(node().name("cold"));
    assert_eq!(text(&provider, &cold, "cpu-percent"), "–");
    assert_eq!(text(&provider, &cold, "cpu"), "–");
}

/// A usage source whose samples move on demand.
struct Sampled(std::sync::atomic::AtomicU64);

impl NodeUsageSource for Sampled {
    fn usage(&self, _: &str) -> Option<NodeUsage> {
        None
    }

    fn epoch(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[test]
fn sorting_by_usage_is_stale_when_a_sample_arrives_unless_the_epoch_moves() {
    let samples = Arc::new(Sampled(0.into()));
    let provider = NodeColumns::new().with_usage(samples.clone());
    let (cpu, memory) = (
        ColumnId::new("cpu-percent"),
        ColumnId::new("memory-percent"),
    );
    let before = (provider.rank_epoch(&cpu), provider.rank_epoch(&memory));
    samples.0.store(1, std::sync::atomic::Ordering::Relaxed);
    assert_ne!(
        provider.rank_epoch(&cpu),
        before.0,
        "a new sample re-ranks CPU%"
    );
    assert_ne!(provider.rank_epoch(&memory), before.1, "and Mem%");
    for absolute in ["cpu", "memory"] {
        assert_eq!(
            provider.rank_epoch(&ColumnId::new(absolute)),
            1,
            "and {absolute}"
        );
    }
    // Columns that read the node alone never ask for a re-rank, and neither does a provider
    // without a usage source.
    assert_eq!(provider.rank_epoch(&ColumnId::new("version")), 0);
    assert_eq!(NodeColumns::new().rank_epoch(&cpu), 0);
}

#[test]
fn the_kind_builds_a_provider_for_each_cluster_and_shows_dashes_without_a_store() {
    use oxikube_domain::ids::{ClusterId, Scope};
    use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};

    use crate::ClusterSessionManager;
    use crate::columns::CoreColumns;
    use crate::kinds::ColumnsContext;
    use crate::nodes::node_columns;

    let meta = KindMeta::new(gvk(), "nodes", "Nodes", Scope::Cluster)
        .with_columns_per_cluster(|context| node_columns(context, None));
    assert!(matches!(meta.columns, KindColumns::PerCluster(_)));
    assert!(meta.columns.provider().is_none());
    let context = ColumnsContext {
        cluster: ClusterId::new("/k", &oxikube_domain::ids::ContextName::new("a")),
        store: None,
        sessions: ClusterSessionManager::new(
            Arc::new(FakeClusterConnectorPort::new()),
            Arc::new(FakeClusterSourcePort::new()),
            Arc::new(FakeClockPort::default()),
        ),
        shared: Arc::new(CoreColumns::new()),
    };
    let (a, b) = (
        meta.columns.build(&context).expect("a provider"),
        meta.columns.build(&context).expect("a provider"),
    );
    assert!(!Arc::ptr_eq(&a, &b));
    let node = object(node().name("n1"));
    assert_eq!(text(&*a, &node, "pods"), "–");
}
