use super::*;

fn pod(names: &[&str]) -> MetricTarget {
    MetricTarget::Pod {
        namespace: "shop".into(),
        pods: PodSelector::Any(names.iter().map(|n| (*n).to_owned()).collect()),
    }
}

#[test]
fn the_support_matrix_has_the_documented_shape() {
    use MetricKind::*;
    for kind in MetricKind::ALL {
        assert!(MetricScope::Cluster.supports(kind), "cluster {kind}");
        assert!(MetricScope::Node.supports(kind), "node {kind}");
        assert!(MetricScope::Namespace.supports(kind), "namespace {kind}");
    }
    let only = |scope: MetricScope, kinds: &[MetricKind]| {
        for kind in MetricKind::ALL {
            assert_eq!(
                scope.supports(kind),
                kinds.contains(&kind),
                "{scope} {kind}"
            );
        }
    };
    only(MetricScope::Pvc, &[DiskUsage, DiskCapacity]);
    only(MetricScope::Ingress, &[NetworkRx, NetworkTx]);
    only(
        MetricScope::Container,
        &[
            CpuUsage,
            CpuRequests,
            CpuLimits,
            MemoryUsage,
            MemoryRequests,
            MemoryLimits,
            DiskUsage,
        ],
    );
    assert!(MetricScope::Pod.supports(NetworkRx) && !MetricScope::Pod.supports(PodCount));
}

#[test]
fn every_kind_has_a_unit_and_an_id() {
    let mut ids: Vec<_> = MetricKind::ALL.iter().map(|k| k.id()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), MetricKind::ALL.len());
    assert_eq!(MetricKind::CpuLimits.unit(), MetricUnit::Cores);
    assert_eq!(MetricKind::NetworkTx.unit(), MetricUnit::BytesPerSecond);
    assert!(MetricKind::MemoryLimits.is_reference() && !MetricKind::MemoryUsage.is_reference());
}

#[test]
fn a_target_knows_its_scope_and_the_label_of_its_series() {
    assert_eq!(MetricTarget::Cluster.scope(), MetricScope::Cluster);
    assert_eq!(pod(&["a"]).series_label(), Some("pod"));
    let containers = MetricTarget::Container {
        namespace: "n".into(),
        pod: "p".into(),
        container: None,
    };
    assert_eq!(containers.scope(), MetricScope::Container);
    assert_eq!(containers.series_label(), Some("container"));
    assert_eq!(MetricTarget::Node("n1".into()).series_label(), None);
}

#[test]
fn validation_names_what_is_wrong() {
    let ok = MetricRequest::new(pod(&["web-0"]), MetricKind::CpuUsage);
    assert_eq!(ok.validate(), Ok(()));

    let unsupported = MetricRequest::new(MetricTarget::Node("n".into()), MetricKind::CpuUsage);
    assert_eq!(unsupported.validate(), Ok(()));
    let pvc_cpu = MetricRequest::new(
        MetricTarget::Pvc {
            namespace: "n".into(),
            name: "data".into(),
        },
        MetricKind::CpuUsage,
    );
    let error = pvc_cpu.validate().unwrap_err();
    assert_eq!(error.to_string(), "no cpu_usage query exists at pvc scope");

    assert_eq!(
        MetricRequest::new(MetricTarget::Node(String::new()), MetricKind::CpuUsage).validate(),
        Err(MetricQueryError::EmptyName("node"))
    );
    assert_eq!(
        MetricRequest::new(pod(&[]), MetricKind::CpuUsage).validate(),
        Err(MetricQueryError::NoPods)
    );
    assert_eq!(
        MetricRequest::new(pod(&["a", ""]), MetricKind::CpuUsage).validate(),
        Err(MetricQueryError::EmptyName("pod"))
    );
}

#[test]
fn label_names_are_validated_because_they_cannot_be_escaped() {
    assert!(LabelMatch::new("cluster", "prod \"eu\"").is_ok());
    assert!(LabelMatch::new("_x9", "v").is_ok());
    for bad in ["", "9a", "a-b", "a b", "a\"}", "é"] {
        assert_eq!(
            LabelMatch::new(bad, "v"),
            Err(MetricQueryError::InvalidLabelName(bad.to_owned())),
            "{bad:?}"
        );
    }
}
