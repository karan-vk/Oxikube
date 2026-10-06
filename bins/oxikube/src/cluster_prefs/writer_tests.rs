//! The posture commands end to end over the real settings store: command bus, guard, handlers,
//! [`SettingsPrefsWriter`], hot reload into the session manager.

use std::sync::Arc;

use gpui::TestAppContext;
use oxikube_app::{
    ClusterSessionManager, CommandBus, CommandRegistry, DispatchContext, MutationGuard, Outcome,
    guard::register_commands,
};
use oxikube_domain::ClusterPreset;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_settings::SettingsStore;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::{SettingsPrefsWriter, follow_cluster_settings};
use crate::app_state::AppState;

const PROD: &str = "3f2a9c1b7d4e8a60";

fn id() -> ClusterId {
    PROD.parse().unwrap()
}

struct Fixture {
    manager: ClusterSessionManager,
    bus: CommandBus,
    state: Arc<FakeStatePort>,
    // Kept alive: the subscription follows the settings, the writer owns the worker task.
    _follow: gpui::Subscription,
    _writer: Arc<SettingsPrefsWriter>,
}

fn fixture(cx: &mut TestAppContext) -> Fixture {
    let context = ClusterContext {
        cluster: id(),
        context: ContextName::new("prod-eu"),
        source: SourceId("kubeconfig".into()),
        server: None,
        default_namespace: None,
    };
    let clock = Arc::new(FakeClockPort::default());
    let manager = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([context.clone()])),
        clock.clone(),
    );
    let state = Arc::new(FakeStatePort::new());
    let (follow, writer) = cx.update(|cx| {
        AppState::test(cx);
        let follow = follow_cluster_settings(cx, manager.clone());
        (follow, Arc::new(SettingsPrefsWriter::new(cx)))
    });
    manager.open_configured(&context);
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_app::posture", |reg| {
            register_commands(reg, manager.clone(), writer.clone())
        })
        .unwrap();
    let guard = MutationGuard::new(manager.clone(), state.clone(), clock);
    Fixture {
        bus: CommandBus::new(registry, guard),
        manager,
        state,
        _follow: follow,
        _writer: writer,
    }
}

fn user_settings(cx: &mut TestAppContext) -> String {
    cx.update(|cx| {
        cx.global::<SettingsStore>()
            .user_settings_text()
            .unwrap_or_default()
            .to_owned()
    })
}

#[gpui::test]
async fn the_production_preset_writes_the_file_and_the_live_session(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let out = f
        .bus
        .dispatch(
            Command::ClusterApplyPreset {
                cluster: id(),
                preset: ClusterPreset::Prod,
            },
            DispatchContext::new(Initiator::Ui, "me"),
        )
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Completed(_)));
    cx.run_until_parked();

    let session = f.manager.get(&id()).unwrap();
    assert!(session.read_only());
    assert_eq!(session.colour(), Some(ClusterPreset::PROD_COLOUR));
    let text = user_settings(cx);
    assert!(text.contains(PROD), "{text}");
    assert!(text.contains("\"read_only\": true"), "{text}");
    assert!(text.contains("#e5484d"), "{text}");
    assert!(
        text.contains("prod-eu"),
        "the new block is named for the context: {text}"
    );
    assert_eq!(f.state.audit_log().len(), 1);
}

#[gpui::test]
async fn lifting_read_only_on_production_confirms_then_edits_the_file(cx: &mut TestAppContext) {
    let f = fixture(cx);
    let ui = || DispatchContext::new(Initiator::Ui, "me");
    f.bus
        .dispatch(
            Command::ClusterApplyPreset {
                cluster: id(),
                preset: ClusterPreset::Prod,
            },
            ui(),
        )
        .await
        .unwrap();
    cx.run_until_parked();

    let lift = Command::ClusterToggleReadOnly {
        cluster: id(),
        read_only: Some(false),
    };
    let request = f
        .bus
        .dispatch(lift.clone(), ui())
        .await
        .unwrap()
        .confirmation()
        .expect("a production cluster asks first");
    assert!(f.manager.get(&id()).unwrap().read_only());

    let out = f
        .bus
        .dispatch(
            lift,
            ui().with_confirmation(oxikube_app::Confirmation::simple(request.token)),
        )
        .await
        .unwrap();
    assert!(matches!(out, Outcome::Completed(_)));
    cx.run_until_parked();
    assert!(!f.manager.get(&id()).unwrap().read_only());
    assert!(user_settings(cx).contains("\"read_only\": false"));
}
