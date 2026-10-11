//! The two bus commands: dispatched through a real `CommandBus`, so the tool stubs, the
//! read-only behaviour and the handlers are the ones the app runs.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::future::BoxFuture;
use oxikube_domain::OxiResult;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{
    ClusterPrefs, ClusterPrefsTable, DetectionResult, DetectionState, MetricsMode,
    PrometheusOverride,
};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeProviderDetectorPort,
    FakeStatePort,
};
use parking_lot::Mutex;

use super::{MetricsProviderService, MetricsSettingsWriter, NoSink, register_commands};
use crate::command_bus::{CommandBus, CommandRegistry, DispatchContext, Outcome};
use crate::guard::MutationGuard;
use crate::session::ClusterSessionManager;
use crate::testing::{cluster_context, ctx, id};

#[derive(Default)]
struct Writer {
    written: Mutex<Vec<(ClusterId, Option<String>, bool)>>,
}

impl MetricsSettingsWriter for Writer {
    fn set_enabled(
        &self,
        cluster: &ClusterId,
        name_hint: Option<&str>,
        enabled: bool,
    ) -> BoxFuture<'static, OxiResult<()>> {
        self.written
            .lock()
            .push((cluster.clone(), name_hint.map(str::to_owned), enabled));
        async { Ok(()) }.boxed()
    }
}

struct Fixture {
    bus: CommandBus,
    sessions: ClusterSessionManager,
    writer: Arc<Writer>,
    detector: Arc<FakeProviderDetectorPort>,
    service: MetricsProviderService,
}

fn fixture(read_only: bool) -> Fixture {
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([cluster_context("a")]));
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    sessions.set_prefs_table(ClusterPrefsTable::new(ClusterPrefs {
        read_only,
        ..ClusterPrefs::default()
    }));
    let detector = Arc::new(FakeProviderDetectorPort::new());
    let service = MetricsProviderService::new(sessions.clone(), detector.clone(), Arc::new(NoSink));
    let writer = Arc::new(Writer::default());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::metrics_provider", |r| {
            register_commands(r, service.clone(), writer.clone())
        })
        .expect("the commands register");
    let guard = MutationGuard::new(sessions.clone(), Arc::new(FakeStatePort::new()), clock);
    Fixture {
        bus: CommandBus::new(registry, guard),
        sessions,
        writer,
        detector,
        service,
    }
}

async fn run(f: &Fixture, command: Command) -> crate::command_bus::CommandOutput {
    match f.bus.dispatch(command, ctx(Initiator::Ui)).await {
        Ok(Outcome::Completed(output)) => output,
        other => panic!("expected a completed command, got {other:?}"),
    }
}

#[test]
fn both_commands_have_tool_stubs_an_agent_may_call() {
    let f = fixture(false);
    for (command, tool) in [
        (
            CommandId::CLUSTER_REDETECT_METRICS,
            "app.cluster_redetect_metrics",
        ),
        (
            CommandId::CLUSTER_TOGGLE_METRICS,
            "app.cluster_toggle_metrics",
        ),
    ] {
        let stub = f.bus.tool(command).expect("a tool stub");
        assert_eq!(stub.name.as_str(), tool);
        assert!(f.bus.agent_tools(false).any(|t| t.name.as_str() == tool));
    }
}

#[tokio::test]
async fn redetect_reports_the_new_status_and_runs_detection() {
    let f = fixture(false);
    f.service
        .start(Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
            tokio::spawn(task);
        }));
    f.sessions.connect(&id("a")).await.unwrap();
    f.detector
        .script()
        .detect
        .push_ok(DetectionResult::of(DetectionState::NotFound))
        .push_ok(DetectionResult::of(DetectionState::NotFound));
    let output = run(&f, Command::ClusterRedetectMetrics { cluster: id("a") }).await;
    let data = output.data.unwrap();
    assert_eq!(data["mode"], "auto");
    assert_eq!(data["detecting"], true);
    assert!(output.message.unwrap().starts_with("Looking for"));
}

#[tokio::test]
async fn redetect_of_an_unopened_cluster_is_an_error_not_a_silent_no_op() {
    let f = fixture(false);
    let error = f
        .bus
        .dispatch(
            Command::ClusterRedetectMetrics { cluster: id("zzz") },
            ctx(Initiator::Ui),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not open"), "{error}");
}

#[tokio::test]
async fn toggling_writes_the_opposite_of_the_current_setting_and_names_the_context() {
    let f = fixture(false);
    f.sessions.open_configured(&cluster_context("a"));
    let output = run(
        &f,
        Command::ClusterToggleMetrics {
            cluster: id("a"),
            enabled: None,
        },
    )
    .await;
    assert_eq!(output.data.unwrap()["enabled"], false);
    assert_eq!(
        f.writer.written.lock().as_slice(),
        [(id("a"), Some("a".to_owned()), false)]
    );

    // With the setting off, a toggle turns it on again; an explicit value wins either way.
    f.sessions
        .set_prefs_table(ClusterPrefsTable::new(ClusterPrefs {
            prometheus: Some(PrometheusOverride {
                enabled: false,
                mode: MetricsMode::Auto,
                ..PrometheusOverride::default()
            }),
            ..ClusterPrefs::default()
        }));
    run(
        &f,
        Command::ClusterToggleMetrics {
            cluster: id("a"),
            enabled: None,
        },
    )
    .await;
    run(
        &f,
        Command::ClusterToggleMetrics {
            cluster: id("a"),
            enabled: Some(false),
        },
    )
    .await;
    let written: Vec<bool> = f.writer.written.lock().iter().map(|w| w.2).collect();
    assert_eq!(written, [false, true, false]);
}

#[tokio::test]
async fn both_commands_run_on_a_read_only_cluster() {
    let f = fixture(true);
    f.sessions.open_configured(&cluster_context("a"));
    assert!(f.sessions.get(&id("a")).unwrap().read_only());
    run(
        &f,
        Command::ClusterToggleMetrics {
            cluster: id("a"),
            enabled: Some(false),
        },
    )
    .await;
    f.service
        .start(Arc::new(|task: futures::future::BoxFuture<'static, ()>| {
            tokio::spawn(task);
        }));
    run(&f, Command::ClusterRedetectMetrics { cluster: id("a") }).await;
}

#[tokio::test]
async fn agents_may_run_them_too() {
    let f = fixture(false);
    f.sessions.open_configured(&cluster_context("a"));
    let outcome = f
        .bus
        .dispatch(
            Command::ClusterToggleMetrics {
                cluster: id("a"),
                enabled: Some(false),
            },
            DispatchContext::new(Initiator::Agent, "agent"),
        )
        .await;
    assert!(matches!(outcome, Ok(Outcome::Completed(_))), "{outcome:?}");
}
