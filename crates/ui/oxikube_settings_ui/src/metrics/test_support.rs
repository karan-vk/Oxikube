//! Helpers for tests of the metrics page and of the crates that mount it (feature
//! `test-support`): a backend serving a status that a test can change, and sample statuses.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc;
use futures::stream::BoxStream;
use oxikube_app::{ProviderState, ProviderStatus};
use oxikube_domain::MetricsProviderKind;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{
    DetectedProvider, DetectionFailure, DetectionState, MetricsMode, Probe, ProbeOutcome,
    PrometheusOverride, ProviderTarget, ServicePort, ServiceRef,
};

use super::backend::{MetricsBackend, PageSource};

/// A cluster id for tests.
pub fn cluster() -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
}

/// A backend with one fixed page that a test replaces with [`set`](Self::set); every `set` wakes
/// the page, as the service does.
#[derive(Clone)]
pub struct ScriptedBackend {
    page: Rc<RefCell<PageSource>>,
    watchers: Rc<RefCell<Vec<mpsc::UnboundedSender<()>>>>,
}

impl ScriptedBackend {
    /// A backend serving `status` for a connected cluster called `kind-oxikube`.
    pub fn new(status: ProviderStatus) -> Self {
        Self {
            page: Rc::new(RefCell::new(PageSource {
                title: "kind-oxikube".into(),
                connected: true,
                status: Arc::new(status),
            })),
            watchers: Rc::default(),
        }
    }

    /// Replaces the status and wakes every page following it.
    pub fn set(&self, status: ProviderStatus) {
        self.page.borrow_mut().status = Arc::new(status);
        self.watchers
            .borrow_mut()
            .retain(|watcher| watcher.unbounded_send(()).is_ok());
    }

    /// Whether the cluster counts as connected.
    pub fn set_connected(&self, connected: bool) {
        self.page.borrow_mut().connected = connected;
    }
}

impl MetricsBackend for ScriptedBackend {
    fn page(&self, _: &ClusterId) -> PageSource {
        self.page.borrow().clone()
    }

    fn changes(&self, _: &ClusterId) -> BoxStream<'static, ()> {
        let (tx, rx) = mpsc::unbounded();
        self.watchers.borrow_mut().push(tx);
        rx.boxed()
    }
}

fn operated() -> ServiceRef {
    ServiceRef::new(
        "monitoring",
        "prometheus-operated",
        ServicePort::Name("web".into()),
    )
}

fn probe(
    namespace: &str,
    service: &str,
    kind: MetricsProviderKind,
    outcome: ProbeOutcome,
) -> Probe {
    Probe {
        namespace: namespace.into(),
        service: service.into(),
        kind,
        outcome,
    }
}

/// kube-prometheus found at `monitoring/prometheus-operated:web`, with the services detection
/// looked at before it.
pub fn found() -> ProviderStatus {
    let mut status = ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::Found(DetectedProvider {
            kind: MetricsProviderKind::KubePrometheus,
            target: ProviderTarget::Service(operated()),
            path_prefix: None,
        })),
    );
    status.probes = vec![
        probe(
            "monitoring",
            "kube-prometheus-stack-prometheus",
            MetricsProviderKind::KubePrometheus,
            ProbeOutcome::NotPresent,
        ),
        probe(
            "monitoring",
            "prometheus-operated",
            MetricsProviderKind::KubePrometheus,
            ProbeOutcome::Answered,
        ),
        probe(
            "lens-metrics",
            "prometheus",
            MetricsProviderKind::Lens,
            ProbeOutcome::NotNeeded,
        ),
    ];
    status
}

/// Every known service looked for and none there.
pub fn not_found() -> ProviderStatus {
    let mut status = ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::NotFound),
    );
    status.probes = vec![
        probe(
            "monitoring",
            "prometheus-operated",
            MetricsProviderKind::KubePrometheus,
            ProbeOutcome::NotPresent,
        ),
        probe(
            "lens-metrics",
            "prometheus",
            MetricsProviderKind::Lens,
            ProbeOutcome::NotPresent,
        ),
        probe(
            "monitoring",
            "vmsingle*",
            MetricsProviderKind::VictoriaMetrics,
            ProbeOutcome::NotPresent,
        ),
    ];
    status
}

/// `list services` was refused everywhere.
pub fn forbidden() -> ProviderStatus {
    let mut status = ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::Failed(DetectionFailure::Forbidden(
            "cannot list services in monitoring, lens-metrics".into(),
        ))),
    );
    status.probes = vec![probe(
        "monitoring",
        "prometheus-operated",
        MetricsProviderKind::KubePrometheus,
        ProbeOutcome::Skipped("you may not list services here".into()),
    )];
    status
}

/// A Service matched a known name and did not answer.
pub fn probe_failed() -> ProviderStatus {
    let mut status = ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::Failed(DetectionFailure::ProbeFailed {
            kind: MetricsProviderKind::KubePrometheus,
            target: ProviderTarget::Service(operated()),
            reason: "the service has no ready endpoint: no endpoints available".into(),
        })),
    );
    status.probes = vec![probe(
        "monitoring",
        "prometheus-operated",
        MetricsProviderKind::KubePrometheus,
        ProbeOutcome::Failed("the service has no ready endpoint".into()),
    )];
    status
}

/// Detection is running.
pub fn detecting() -> ProviderStatus {
    ProviderStatus::new(
        MetricsMode::Auto,
        ProviderState::Detection(DetectionState::Detecting),
    )
}

/// The settings turn metrics off.
pub fn disabled() -> ProviderStatus {
    ProviderStatus::new(MetricsMode::Off, ProviderState::Disabled).with_settings(
        PrometheusOverride {
            enabled: false,
            ..PrometheusOverride::default()
        },
    )
}

/// A VictoriaMetrics endpoint configured by hand that answered.
pub fn manual() -> ProviderStatus {
    let settings = PrometheusOverride {
        mode: MetricsMode::Manual,
        kind: Some(MetricsProviderKind::VictoriaMetrics),
        url: Some("https://vm.example.com".into()),
        path_prefix: Some("/select/0/prometheus".into()),
        auth: oxikube_ports::PrometheusAuthKind::Bearer,
        auth_secret: oxikube_ports::SecretKey::new("prometheus", "prod-vm").ok(),
        ..PrometheusOverride::default()
    };
    ProviderStatus::new(
        MetricsMode::Manual,
        ProviderState::Detection(DetectionState::Found(DetectedProvider {
            kind: MetricsProviderKind::VictoriaMetrics,
            target: ProviderTarget::Url("https://vm.example.com".into()),
            path_prefix: Some("/select/0/prometheus".into()),
        })),
    )
    .with_settings(settings)
}

/// A [`CommandDispatcher`](oxikube_workspace::CommandDispatcher) that records every command: the
/// fake bus of the page's tests. Cheap to clone; clones share the record.
#[derive(Clone, Default)]
pub struct RecordingDispatcher {
    sent: Rc<RefCell<Vec<oxikube_domain::command::Command>>>,
}

impl RecordingDispatcher {
    /// A dispatcher with nothing recorded.
    pub fn new() -> Self {
        Self::default()
    }

    /// The commands sent so far, oldest first.
    pub fn sent(&self) -> Vec<oxikube_domain::command::Command> {
        self.sent.borrow().clone()
    }
}

impl oxikube_workspace::CommandDispatcher for RecordingDispatcher {
    fn dispatch(&self, command: oxikube_domain::command::Command, _: &mut gpui::App) {
        self.sent.borrow_mut().push(command);
    }
}
