//! The dialog as a view: a late annotation preselect and a failed pod read must not undo what the
//! user typed or the ports the dialog was opened with.

use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext, WeakEntity};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::portforward::ObjectWatch;
use oxikube_app::store::{Spawner, StoreQuery};
use oxikube_app::{
    AnnotationSuggestion, ClusterSessionManager, ForwardBackend, ForwardStarter, MutationGuard,
    PortForwardManager, SuggestedForward,
};
use oxikube_domain::ids::ClusterId;
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};

use super::target;
use crate::dialog::{PortChoice, StartDialog, StartForm};

struct Backend;

impl ForwardBackend for Backend {
    fn port_forward(&self, _: &ClusterId) -> Option<Arc<dyn oxikube_ports::PortForwardPort>> {
        None
    }

    fn is_read_only(&self, _: &ClusterId) -> bool {
        false
    }

    fn watch(&self, _: &ClusterId, _: StoreQuery) -> Option<ObjectWatch> {
        None
    }
}

fn starter() -> ForwardStarter {
    starter_with(oxikube_app::ForwardConfig::default())
}

fn starter_with(config: oxikube_app::ForwardConfig) -> ForwardStarter {
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new()),
        clock.clone(),
    );
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock.clone());
    let bus = CommandBus::new(CommandRegistry::new(), guard);
    let spawner: Arc<dyn Spawner> = Arc::new(|_: futures::future::BoxFuture<'static, ()>| {});
    let manager = PortForwardManager::new(Arc::new(Backend), spawner, clock, None);
    manager.set_config(config);
    ForwardStarter::new(bus, "alice", manager)
}

fn suggestion() -> AnnotationSuggestion {
    AnnotationSuggestion {
        forward: SuggestedForward {
            container: None,
            local_port: 8080,
            remote_port: 80,
            auto: true,
        },
        conflict: None,
    }
}

fn dialog(
    ports: Vec<PortChoice>,
    cx: &mut TestAppContext,
) -> (Entity<StartDialog>, &mut VisualTestContext) {
    dialog_with(ports, starter(), cx)
}

fn dialog_with(
    ports: Vec<PortChoice>,
    starter: ForwardStarter,
    cx: &mut TestAppContext,
) -> (Entity<StartDialog>, &mut VisualTestContext) {
    let (dialog, cx) = cx.add_window_view(|window, cx| {
        oxikube_ui::init(cx);
        StartDialog::new(
            StartForm::new(target("Pod", "web"), ports),
            None,
            starter,
            WeakEntity::new_invalid(),
            window,
            cx,
        )
    });
    (dialog, cx)
}

fn text(
    dialog: &Entity<StartDialog>,
    cx: &mut VisualTestContext,
    field: fn(&StartDialog) -> &Entity<oxikube_ui::input::InputState>,
) -> String {
    dialog.read_with(cx, |d, cx| field(d).read(cx).value().to_string())
}

#[gpui::test]
fn a_late_preselect_leaves_what_the_user_typed(cx: &mut TestAppContext) {
    let (dialog, cx) = dialog(Vec::new(), cx);
    dialog.update_in(cx, |d, window, cx| {
        d.local
            .update(cx, |input, cx| input.set_value("9000", window, cx));
    });
    cx.run_until_parked();
    dialog.update_in(cx, |d, window, cx| {
        d.set_suggestions(vec![suggestion()], window, cx);
    });
    assert_eq!(text(&dialog, cx, |d| &d.local), "9000");
    assert_eq!(text(&dialog, cx, |d| &d.remote), "");
}

#[gpui::test]
fn a_late_preselect_fills_untouched_fields(cx: &mut TestAppContext) {
    let (dialog, cx) = dialog(Vec::new(), cx);
    dialog.update_in(cx, |d, window, cx| {
        d.set_suggestions(vec![suggestion()], window, cx);
    });
    assert_eq!(text(&dialog, cx, |d| &d.local), "8080");
    assert_eq!(text(&dialog, cx, |d| &d.remote), "80");
}

#[gpui::test]
fn a_failed_read_keeps_the_ports_the_dialog_was_opened_with(cx: &mut TestAppContext) {
    let port = PortChoice {
        number: 8080,
        name: None,
        container: None,
    };
    let (dialog, cx) = dialog(vec![port.clone()], cx);
    dialog.update_in(cx, |d, window, cx| {
        // What `load_ports` does when the read fails.
        let kept = d.form.ports.clone();
        d.set_ports(kept, window, cx);
    });
    dialog.read_with(cx, |d, _| {
        assert_eq!(d.form.ports, vec![port]);
        assert!(!d.loading_ports);
    });
}

/// Audit E15 (#18): the Bind field always showed 127.0.0.1, whatever `portforward.default_bind`.
#[gpui::test]
fn the_bind_field_starts_from_the_default_bind_setting(cx: &mut TestAppContext) {
    let config = oxikube_app::ForwardConfig {
        default_bind: "0.0.0.0".parse().unwrap(),
        ..Default::default()
    };
    let (dialog, cx) = dialog_with(Vec::new(), starter_with(config), cx);
    assert_eq!(text(&dialog, cx, |d| &d.bind), "0.0.0.0");
}

#[gpui::test]
fn a_bind_the_user_chose_over_the_setting_is_remembered(cx: &mut TestAppContext) {
    cx.update(|cx| crate::dialog::prefs::LastUsed::set(Some("::1".into()), true, cx));
    let (dialog, cx) = dialog(Vec::new(), cx);
    assert_eq!(text(&dialog, cx, |d| &d.bind), "::1");
}

#[gpui::test]
fn without_a_choice_the_bind_field_shows_loopback(cx: &mut TestAppContext) {
    let (dialog, cx) = dialog(Vec::new(), cx);
    assert_eq!(text(&dialog, cx, |d| &d.bind), "127.0.0.1");
}
