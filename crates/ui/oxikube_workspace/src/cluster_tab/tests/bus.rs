//! The tab commands on the `CommandBus`: registered, with tool stubs, and running the UI.

use gpui::TestAppContext;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext, Outcome};
use oxikube_app::guard::MutationGuard;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::CommandId;

use super::*;
use crate::cluster_tab::{TabsDispatcher, register_commands};

const TAB_COMMANDS: [(CommandId, &str); 5] = [
    (CommandId::CLUSTER_SELECT, "app.cluster_select"),
    (CommandId::CLUSTER_SWITCH_TAB, "app.cluster_switch_tab"),
    (CommandId::CLUSTER_NEXT_TAB, "app.cluster_next_tab"),
    (CommandId::CLUSTER_PREVIOUS_TAB, "app.cluster_previous_tab"),
    (CommandId::CLUSTER_CLOSE_TAB, "app.cluster_close_tab"),
];

fn bus(fx: &mut Fixture) -> CommandBus {
    let sink = fx.vcx.update(|_, cx| fx.tabs.read(cx).command_sink());
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_workspace", |registry| {
            register_commands(registry, sink)
        })
        .expect("the tab commands register");
    let guard = MutationGuard::new(
        fx.sessions.clone(),
        fx.state.clone(),
        Arc::new(FakeClockPort::default()),
    );
    CommandBus::new(registry, guard)
}

#[gpui::test]
fn every_tab_command_is_registered_with_a_tool_stub(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    let bus = bus(&mut fx);
    for (id, tool) in TAB_COMMANDS {
        assert!(bus.is_registered(id), "{id}");
        assert_eq!(bus.owner(id), Some("oxikube_workspace"));
        let stub = bus
            .tool(id)
            .unwrap_or_else(|| panic!("{id} has a tool stub"));
        assert_eq!(stub.name.as_str(), tool);
        // None of them changes a cluster.
        assert!(!id.as_str().is_empty() && !bus.commands().any(|m| m.id == id && m.mutating));
    }
    // A second registration of the same ids is refused: one owner per command.
    let sink = fx.vcx.update(|_, cx| fx.tabs.read(cx).command_sink());
    let mut registry = CommandRegistry::new();
    registry
        .install("one", |r| register_commands(r, sink.clone()))
        .expect("first");
    assert!(
        registry
            .install("two", |r| register_commands(r, sink))
            .is_err()
    );
}

#[gpui::test]
fn dispatching_through_the_bus_switches_the_tabs(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta", "gamma"]);
    for name in ["alpha", "beta", "gamma"] {
        fx.connect(name);
    }
    let bus = bus(&mut fx);
    let ctx = || DispatchContext::new(Initiator::Command, "me");
    let run = |fx: &mut Fixture, command: Command| {
        let outcome = block_on(bus.dispatch(command, ctx())).expect("dispatch");
        assert!(matches!(outcome, Outcome::Completed(_)));
        fx.vcx.run_until_parked();
    };

    run(&mut fx, Command::ClusterNextTab);
    assert_eq!(fx.active_name().as_deref(), Some("alpha"));
    run(&mut fx, Command::ClusterSwitchTab { index: 2 });
    assert_eq!(fx.active_name().as_deref(), Some("beta"));
    run(&mut fx, Command::ClusterPreviousTab);
    assert_eq!(fx.active_name().as_deref(), Some("alpha"));
    run(
        &mut fx,
        Command::ClusterSelect {
            cluster: id("gamma"),
        },
    );
    assert_eq!(fx.active_name().as_deref(), Some("gamma"));
    run(
        &mut fx,
        Command::ClusterCloseTab {
            cluster: id("gamma"),
        },
    );
    assert_eq!(fx.open_names(), ["alpha", "beta"]);
    assert_eq!(fx.recorder.disconnects(), 1);
}

#[gpui::test]
fn an_agent_may_switch_tabs_too(cx: &mut TestAppContext) {
    // The tab commands are not privileged: the MCP tools exist for every initiator.
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    let bus = bus(&mut fx);
    block_on(bus.dispatch(
        Command::ClusterSelect {
            cluster: id("alpha"),
        },
        DispatchContext::new(Initiator::Agent, "claude"),
    ))
    .expect("an agent may select a cluster");
    fx.vcx.run_until_parked();
    assert_eq!(fx.active_name().as_deref(), Some("alpha"));
}

#[gpui::test]
fn the_tabs_dispatcher_keeps_tab_commands_and_forwards_the_rest(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    let sink = fx.vcx.update(|_, cx| fx.tabs.read(cx).command_sink());
    let dispatcher = TabsDispatcher::new(Rc::new(fx.recorder.clone()), sink);
    fx.vcx.update(|_, cx| {
        dispatcher.dispatch(
            Command::ClusterSelect {
                cluster: id("alpha"),
            },
            cx,
        );
        dispatcher.dispatch(
            Command::ClusterToggleFavourite {
                cluster: id("beta"),
                favourite: None,
            },
            cx,
        );
    });
    fx.vcx.run_until_parked();
    assert_eq!(fx.active_name().as_deref(), Some("alpha"));
    assert_eq!(
        fx.recorder.sent(),
        [Command::ClusterToggleFavourite {
            cluster: id("beta"),
            favourite: None
        }],
        "only what the tabs do not run reaches the wrapped dispatcher"
    );
}
