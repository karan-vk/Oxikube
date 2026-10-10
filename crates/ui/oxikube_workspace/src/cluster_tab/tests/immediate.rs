//! The tab commands are immediate (E05-P600): run by the command runner or a key, the other tab
//! is shown in the update that took the input, before any executor turn, so the frame drawn right
//! after it shows the switch.

use gpui::{Keystroke, TestAppContext};
use oxikube_app::command_bus::{CommandBus, CommandRegistry};
use oxikube_app::guard::MutationGuard;
use oxikube_domain::command::CommandId;

use super::*;
use crate::cluster::ClusterCommandRunner;
use crate::cluster_tab::{TabsDispatcher, register_commands};

fn three(cx: &mut TestAppContext) -> Fixture {
    let mut fx = Fixture::open(cx, &["alpha", "beta", "gamma"]);
    for name in ["alpha", "beta", "gamma"] {
        fx.connect(name);
    }
    fx
}

fn runner(fx: &mut Fixture) -> ClusterCommandRunner {
    let sink = fx.vcx.update(|_, cx| fx.tabs.read(cx).command_sink());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_workspace", |r| register_commands(r, sink))
        .expect("the tab commands register");
    let bus = CommandBus::new(
        registry,
        MutationGuard::new(
            fx.sessions.clone(),
            fx.state.clone(),
            Arc::new(FakeClockPort::default()),
        ),
    );
    for id in [
        CommandId::CLUSTER_SELECT,
        CommandId::CLUSTER_SWITCH_TAB,
        CommandId::CLUSTER_NEXT_TAB,
        CommandId::CLUSTER_PREVIOUS_TAB,
        CommandId::CLUSTER_CLOSE_TAB,
    ] {
        assert!(bus.runs_now(id), "{id} is immediate");
    }
    ClusterCommandRunner::new(bus, fx.sessions.clone(), "me", &fx.ws)
}

/// The cluster tab the window's workspace displays: what the next frame draws. (The controller's
/// own `active` follows a turn later: the dock tells a tab it is shown from a task.)
fn shown(fx: &mut Fixture) -> Option<String> {
    let (ws, tabs) = (fx.ws.clone(), fx.tabs.clone());
    fx.vcx.update(|_, cx| active_title(&ws, &tabs, cx))
}

#[gpui::test]
fn the_runner_switches_the_tab_in_the_dispatching_update(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    let runner = runner(&mut fx);
    let tabs = fx.tabs.clone();
    let ws = fx.ws.clone();

    let (before, inside) = fx.vcx.update(|window, cx| {
        let before = active_title(&ws, &tabs, cx);
        runner.run(Command::ClusterNextTab, window, cx);
        // Still inside the dispatching update: the window's workspace already shows it.
        (before, active_title(&ws, &tabs, cx))
    });
    assert_eq!(before.as_deref(), Some("gamma"));
    assert_eq!(inside.as_deref(), Some("alpha"), "shown in the same update");

    fx.vcx
        .update(|window, cx| runner.run(Command::ClusterSwitchTab { index: 2 }, window, cx));
    assert_eq!(shown(&mut fx).as_deref(), Some("beta"));
    fx.vcx.update(|window, cx| {
        runner.run(
            Command::ClusterSelect {
                cluster: id("gamma"),
            },
            window,
            cx,
        )
    });
    assert_eq!(shown(&mut fx).as_deref(), Some("gamma"));

    // Nothing was left for the controller's task, and the controller agrees once told.
    fx.vcx.run_until_parked();
    assert_eq!(shown(&mut fx).as_deref(), Some("gamma"));
    assert_eq!(fx.active_name().as_deref(), Some("gamma"));
}

#[gpui::test]
fn two_steps_in_one_update_step_twice(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    let runner = runner(&mut fx);
    fx.vcx.update(|window, cx| {
        runner.run(Command::ClusterNextTab, window, cx);
        runner.run(Command::ClusterNextTab, window, cx);
    });
    assert_eq!(
        shown(&mut fx).as_deref(),
        Some("beta"),
        "gamma, alpha, beta"
    );
    fx.vcx.run_until_parked();
    assert_eq!(fx.active_name().as_deref(), Some("beta"));
}

#[gpui::test]
fn ctrl_tab_switches_the_tab_before_the_next_frame(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    // One key, dispatched as the platform does, and no executor turn after it.
    fx.vcx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("ctrl-tab").unwrap(), cx);
    });
    assert_eq!(shown(&mut fx).as_deref(), Some("alpha"));
    fx.vcx.update(|window, cx| {
        window.dispatch_keystroke(Keystroke::parse("ctrl-shift-tab").unwrap(), cx);
    });
    assert_eq!(shown(&mut fx).as_deref(), Some("gamma"));
}

#[gpui::test]
fn a_hotbar_click_switches_the_tab_before_the_next_frame(cx: &mut TestAppContext) {
    let mut fx = three(cx);
    fx.vcx.update(|window, _| window.activate_window());
    let sink = fx.vcx.update(|_, cx| fx.tabs.read(cx).command_sink());
    let dispatcher = TabsDispatcher::new(Rc::new(fx.recorder.clone()), sink);
    fx.vcx.update(|_, cx| {
        dispatcher.dispatch(
            Command::ClusterSelect {
                cluster: id("beta"),
            },
            cx,
        );
    });
    // The update has ended, no executor turn yet.
    assert_eq!(shown(&mut fx).as_deref(), Some("beta"));
}

fn active_title(
    ws: &gpui::Entity<Workspace>,
    tabs: &gpui::Entity<ClusterTabs>,
    cx: &gpui::App,
) -> Option<String> {
    let item = ws.read(cx).active_item(cx)?.item_id();
    let tabs = tabs.read(cx);
    tabs.clusters(cx).into_iter().find_map(|cluster| {
        let tab = tabs.tab(&cluster)?;
        (tab.entity_id() == item).then(|| tab.read(cx).info().title.to_string())
    })
}
