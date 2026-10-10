//! One `Fake*` per port trait in `oxikube_ports`, with scripted responses and recorded
//! calls.
//!
//! Every fake has the same shape:
//!
//! * `fake.script().<method>` is a [`Script`](crate::Script) of queued responses for that
//!   method (`push_ok`, `push_err`); each call pops one.
//! * `fake.recorded_calls()` returns every call made so far as a `*Call` enum, in order,
//!   with its arguments cloned (secret values are never recorded).
//! * When a method's queue is empty the fake falls back to its configured state (an
//!   in-memory store, a configured list) or, where there is none, returns an `Internal`
//!   error naming the port and method ([`unscripted`](crate::unscripted)). Each fake's
//!   rustdoc lists its fallbacks. A scripted response replaces the fallback for that call
//!   entirely: it has no side effect on the fake's state.
//!
//! Streaming methods (`watch`, `table_feed`, `stream_logs`, `updates`) are scripted with a
//! [`Timeline`](crate::Timeline) replayed against the fake's [`FakeClockPort`], so delivery
//! timing is deterministic and nothing sleeps for real. No fake starts an OS thread or
//! needs tokio.
//!
//! | Port | Fake |
//! |---|---|
//! | `ResourceReader` + `ResourceWriter` (`ResourcePort`) | [`FakeResourcePort`] |
//! | `DiscoveryPort` | [`FakeDiscoveryPort`] |
//! | `TableFeedPort` | [`FakeTableFeedPort`] |
//! | `LogPort` | [`FakeLogPort`] |
//! | `ExecPort` (+ `TerminalBackend`) | [`FakeExecPort`] (+ [`FakeTerminalBackend`]) |
//! | `ExecStreamPort` | [`FakeExecStreamPort`] |
//! | `PortForwardPort` | [`FakePortForwardPort`] |
//! | `ClusterSourcePort` | [`FakeClusterSourcePort`] |
//! | `CloudDiscoveryPort` | [`FakeCloudDiscoveryPort`] |
//! | `MetricsPort` | [`FakeMetricsPort`] |
//! | `PromqlPort` | [`FakePromqlPort`] |
//! | `DescribePort` | [`FakeDescribePort`] |
//! | `HelmPort` | [`FakeHelmPort`] |
//! | `StatePort` | [`FakeStatePort`] |
//! | `SecretStorePort` | [`FakeSecretStorePort`] |
//! | `NotifierPort` | [`FakeNotifierPort`] |
//! | `UpdaterPort` | [`FakeUpdaterPort`] |
//! | `CrashReporterPort` | [`FakeCrashReporterPort`] |
//! | `FsPort` | [`FakeFsPort`] |
//! | `ClockPort` | [`FakeClockPort`] |
//! | `IntegrationPort` | [`FakeIntegrationPort`] |
//! | `ToolPort` | [`FakeToolPort`] |
//! | `ContextProviderPort` | [`FakeContextProviderPort`] |
//! | `AgentPort` | [`FakeAgentPort`] |
//! | `AgentClient` | [`FakeAgentClient`] |
//! | `ClusterConnectorPort` | [`FakeClusterConnectorPort`] (ports bundle: [`FakeClusterPorts`]) |
//! | `AccessReviewPort` | [`FakeAccessReviewPort`] |
//! | `WarningPort` | [`FakeWarningPort`] |
//! | `SchemaPort` | [`FakeSchemaPort`] |

/// Implements `script()`, `recorded_calls()` and `clear_calls()` for a fake with fields
/// `script: $scripts` and `calls: CallLog<$call>`.
macro_rules! fake_plumbing {
    ($fake:ty, $scripts:ty, $call:ty) => {
        impl $fake {
            /// The per-method response queues. Push responses before calling the port.
            pub fn script(&self) -> &$scripts {
                &self.script
            }

            /// Every call made on this fake so far, in call order.
            pub fn recorded_calls(&self) -> Vec<$call> {
                self.calls.calls()
            }

            /// Forgets the recorded calls.
            pub fn clear_calls(&self) {
                self.calls.clear();
            }
        }
    };
}

mod agent;
mod clock;
mod cluster;
mod data;
mod infra;
mod integration;
mod resource;
mod schema;
mod session;
mod storage;
mod stream_io;
mod terminal;
mod warnings;

pub use agent::{
    AgentCall, AgentClientCall, AgentClientScripts, AgentScripts, FakeAgentClient, FakeAgentPort,
};
pub use clock::{ClockCall, DEFAULT_START, FakeClockPort};
pub use cluster::{
    CloudCall, CloudScripts, ClusterSourceCall, ClusterSourceScripts, FakeCloudDiscoveryPort,
    FakeClusterSourcePort,
};
pub use data::{
    DiscoveryCall, DiscoveryScripts, FakeDiscoveryPort, FakeLogPort, FakeTableFeedPort, LogCall,
    LogScripts, TableCall, TableScripts,
};
pub use infra::{
    CrashCall, CrashScripts, DescribeCall, DescribeScripts, FakeCrashReporterPort,
    FakeDescribePort, FakeHelmPort, FakeMetricsPort, FakeNotifierPort, FakePromqlPort,
    FakeUpdaterPort, HelmCall, HelmScripts, MetricsCall, MetricsScripts, NotifierCall,
    NotifierScripts, PromqlCall, PromqlScripts, UpdaterCall, UpdaterScripts,
};
pub use integration::{
    ContextProviderCall, ContextProviderScripts, FakeContextProviderPort, FakeIntegrationPort,
    FakeToolPort, IntegrationCall, IntegrationScripts, ToolCall, ToolScripts,
};
pub use resource::{FakeResourcePort, ResourceCall, ResourceScripts};
pub use schema::{FakeSchemaPort, SchemaCall, SchemaScripts};
pub use session::{
    AccessCall, AccessScripts, ConnectorCall, ConnectorScripts, FakeAccessReviewPort,
    FakeClusterConnectorPort, FakeClusterPorts,
};
pub use storage::{
    FakeFsPort, FakeSecretStorePort, FakeStatePort, FsCall, FsScripts, SecretCall, SecretScripts,
    StateCall, StateScripts,
};
pub use stream_io::{
    ExecCapture, ExecScript, ExecStreamCall, ExecStreamScripts, FakeExecStreamPort,
    FakePortForwardPort, ForwardCapture, ForwardScript, PortForwardCall, PortForwardScripts,
};
pub use terminal::{
    ExecPortCall, ExecPortScripts, FakeExecPort, FakeTerminalBackend, TerminalCall,
};
pub use warnings::FakeWarningPort;
