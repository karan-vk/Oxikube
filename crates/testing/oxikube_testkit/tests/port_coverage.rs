//! Every port trait in `oxikube_ports` has a fake here ("every port has a fake; if you add a
//! port, add its fake in the same PR"): a compile-time check that each fake implements its
//! port as a trait object, and a source scan that fails when a new port trait appears
//! without one.

use std::path::Path;
use std::sync::Arc;

use oxikube_ports::{
    AccessReviewPort, AgentClient, AgentPort, ClockPort, CloudDiscoveryPort, ClusterConnectorPort,
    ClusterSourcePort, ContextProviderPort, CrashReporterPort, DescribePort, DiscoveryPort,
    ExecPort, ExecStreamPort, FsPort, HelmPort, IntegrationPort, LogPort, MetricsPort,
    NotifierPort, PortForwardPort, PromqlPort, ResourcePort, ResourceReader, ResourceWriter,
    SchemaPort, SecretStorePort, StatePort, TableFeedPort, TerminalBackend, ToolDef, ToolName,
    ToolPort, UpdaterPort, WarningPort,
};
use oxikube_testkit::*;

/// `(port trait, fake type)`; each pair is also checked by `fakes_are_port_trait_objects`.
const FAKES: &[(&str, &str)] = &[
    ("ResourceReader", "FakeResourcePort"),
    ("ResourceWriter", "FakeResourcePort"),
    ("DiscoveryPort", "FakeDiscoveryPort"),
    ("TableFeedPort", "FakeTableFeedPort"),
    ("LogPort", "FakeLogPort"),
    ("ExecPort", "FakeExecPort"),
    ("ExecStreamPort", "FakeExecStreamPort"),
    ("TerminalBackend", "FakeTerminalBackend"),
    ("PortForwardPort", "FakePortForwardPort"),
    ("ClusterSourcePort", "FakeClusterSourcePort"),
    ("CloudDiscoveryPort", "FakeCloudDiscoveryPort"),
    ("MetricsPort", "FakeMetricsPort"),
    ("PromqlPort", "FakePromqlPort"),
    ("DescribePort", "FakeDescribePort"),
    ("HelmPort", "FakeHelmPort"),
    ("StatePort", "FakeStatePort"),
    ("SecretStorePort", "FakeSecretStorePort"),
    ("NotifierPort", "FakeNotifierPort"),
    ("UpdaterPort", "FakeUpdaterPort"),
    ("CrashReporterPort", "FakeCrashReporterPort"),
    ("FsPort", "FakeFsPort"),
    ("ClockPort", "FakeClockPort"),
    ("IntegrationPort", "FakeIntegrationPort"),
    ("ToolPort", "FakeToolPort"),
    ("ContextProviderPort", "FakeContextProviderPort"),
    ("AgentPort", "FakeAgentPort"),
    ("AgentClient", "FakeAgentClient"),
    ("ClusterConnectorPort", "FakeClusterConnectorPort"),
    ("AccessReviewPort", "FakeAccessReviewPort"),
    ("WarningPort", "FakeWarningPort"),
    ("SchemaPort", "FakeSchemaPort"),
];

/// Traits in `oxikube_ports` that are not ports: blanket combinations, extensions, and
/// callbacks the app implements for adapters (`HealthReporter`).
const NOT_PORTS: &[&str] = &[
    "ResourcePort",
    "StatePortExt",
    "DuplexStream",
    "HealthReporter",
];

#[test]
fn fakes_are_port_trait_objects() {
    let tool = ToolDef::read_only(
        ToolName::new("test.noop").unwrap(),
        "No-op",
        serde_json::json!({"type": "object"}),
    );
    let resource = Arc::new(FakeResourcePort::new());
    let _: Arc<dyn ResourcePort> = resource.clone();
    let _: Arc<dyn ResourceReader> = resource.clone();
    let _: Arc<dyn ResourceWriter> = resource;
    let _: Arc<dyn DiscoveryPort> = Arc::new(FakeDiscoveryPort::new());
    let _: Arc<dyn TableFeedPort> = Arc::new(FakeTableFeedPort::new());
    let _: Arc<dyn LogPort> = Arc::new(FakeLogPort::new());
    let _: Arc<dyn ExecPort> = Arc::new(FakeExecPort::new());
    let _: Arc<dyn ExecStreamPort> = Arc::new(FakeExecStreamPort::new());
    let _: Box<dyn TerminalBackend> = Box::new(FakeTerminalBackend::echo());
    let _: Arc<dyn PortForwardPort> = Arc::new(FakePortForwardPort::new());
    let _: Arc<dyn ClusterSourcePort> = Arc::new(FakeClusterSourcePort::new());
    let _: Arc<dyn CloudDiscoveryPort> = Arc::new(FakeCloudDiscoveryPort::new());
    let _: Arc<dyn MetricsPort> = Arc::new(FakeMetricsPort::new());
    let _: Arc<dyn PromqlPort> = Arc::new(FakePromqlPort::new());
    let _: Arc<dyn DescribePort> = Arc::new(FakeDescribePort::new());
    let _: Arc<dyn HelmPort> = Arc::new(FakeHelmPort::new());
    let _: Arc<dyn StatePort> = Arc::new(FakeStatePort::new());
    let _: Arc<dyn SecretStorePort> = Arc::new(FakeSecretStorePort::new());
    let _: Arc<dyn NotifierPort> = Arc::new(FakeNotifierPort::new());
    let _: Arc<dyn UpdaterPort> = Arc::new(FakeUpdaterPort::new());
    let _: Arc<dyn CrashReporterPort> = Arc::new(FakeCrashReporterPort::new());
    let _: Arc<dyn FsPort> = Arc::new(FakeFsPort::new());
    let _: Arc<dyn ClockPort> = Arc::new(FakeClockPort::default());
    let _: Arc<dyn IntegrationPort> = Arc::new(FakeIntegrationPort::new("x"));
    let _: Arc<dyn ToolPort> = Arc::new(FakeToolPort::new(tool));
    let _: Arc<dyn ContextProviderPort> = Arc::new(FakeContextProviderPort::new("x"));
    let _: Arc<dyn AgentPort> = Arc::new(FakeAgentPort::new());
    let _: Arc<dyn AgentClient> = Arc::new(FakeAgentClient::new());
    let _: Arc<dyn ClusterConnectorPort> = Arc::new(FakeClusterConnectorPort::new());
    let _: Arc<dyn AccessReviewPort> = Arc::new(FakeAccessReviewPort::new());
    let _: Arc<dyn WarningPort> = Arc::new(FakeWarningPort::new());
    let _: Arc<dyn SchemaPort> = Arc::new(FakeSchemaPort::new());
}

/// Every `pub trait` declared in `oxikube_ports/src` is either a port with a listed fake
/// or a known non-port.
#[test]
fn every_port_trait_in_oxikube_ports_has_a_fake() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ports/oxikube_ports/src");
    let mut traits = Vec::new();
    let mut dirs = vec![src];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for line in text.lines() {
                if let Some(rest) = line.trim_start().strip_prefix("pub trait ") {
                    let name: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    traits.push(name);
                }
            }
        }
    }
    assert!(traits.len() >= FAKES.len(), "found only {traits:?}");
    for name in &traits {
        if NOT_PORTS.contains(&name.as_str()) {
            continue;
        }
        assert!(
            FAKES.iter().any(|(port, _)| port == name),
            "port trait `{name}` has no fake in oxikube_testkit"
        );
    }
    for (port, _) in FAKES {
        assert!(
            traits.iter().any(|t| t == port),
            "`{port}` is no longer a port trait"
        );
    }
}
