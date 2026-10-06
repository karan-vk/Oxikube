//! What the buttons do: every one sends a `Command` (or runs a host hook), and nothing else.

use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::session::SessionPhase;
use oxikube_ports::HealthSignal;

use super::fixture::{Dispatch, Fixture, Offers};
use super::id;

fn auth_required(fx: &mut Fixture, name: &str) {
    fx.connector
        .connect_script_for(&id(name))
        .push_err(OxiError::auth("token expired", false));
    fx.connect(name);
    assert_eq!(fx.phase(name), SessionPhase::AuthRequired);
}

fn failed(fx: &mut Fixture, name: &str) {
    fx.connector
        .connect_script_for(&id(name))
        .push_err(OxiError::validation("context has no cluster entry"));
    fx.connect(name);
    assert_eq!(fx.phase(name), SessionPhase::Error);
}

#[gpui::test]
fn retry_is_the_reconnect_command_in_every_state_that_has_it(cx: &mut TestAppContext) {
    let reconnect = || {
        [Command::ClusterReconnect {
            cluster: id("prod-eu"),
        }]
    };

    let mut fx = Fixture::open(cx, &["prod-eu"]);
    auth_required(&mut fx, "prod-eu");
    fx.click("connect-retry");
    assert_eq!(fx.recorder.sent(), reconnect(), "AuthRequired");

    let mut fx = Fixture::open(cx, &["prod-eu"]);
    failed(&mut fx, "prod-eu");
    fx.click("connect-retry");
    assert_eq!(fx.recorder.sent(), reconnect(), "Error");

    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connect("prod-eu");
    fx.connector.report(&id("prod-eu"), HealthSignal::Unhealthy);
    fx.vcx.run_until_parked();
    fx.click("connect-banner-retry");
    assert_eq!(fx.recorder.sent(), reconnect(), "Degraded");
}

#[gpui::test]
fn cancel_is_the_cancel_connect_command(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.begin_connect("prod-eu");
    fx.click("connect-cancel");
    assert_eq!(
        fx.recorder.sent(),
        [Command::ClusterCancelConnect {
            cluster: id("prod-eu")
        }]
    );
}

#[gpui::test]
fn cancelling_for_real_stops_the_attempt_and_closes_the_tab(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(cx, &["prod-eu"], Dispatch::Run, Offers::default());
    fx.begin_connect("prod-eu");
    assert!(fx.drawn("connect-spinner"));
    fx.click("connect-cancel");
    assert_eq!(fx.phase("prod-eu"), SessionPhase::Disconnected);
    let tabs = fx.tabs.clone();
    assert!(
        fx.vcx.update(|_, cx| tabs.read(cx).is_empty()),
        "no tab for a closed session"
    );
    assert!(!fx.drawn("connect-spinner"));
}

#[gpui::test]
fn connect_from_the_disconnected_body_is_the_connect_command(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    fx.connect("prod-eu");
    let view = fx.view("prod-eu");
    // The session ends (the tab goes with it) while the view is still alive: it offers Connect.
    fx.vcx
        .update(|_, cx| view.update(cx, |view, cx| view.connect(cx)));
    assert_eq!(
        fx.recorder.sent(),
        [Command::ClusterConnect {
            cluster: id("prod-eu")
        }]
    );
}

#[test]
fn the_commands_are_reads_with_mcp_tool_stubs() {
    let cluster = id("prod-eu");
    for (command, expected, tool) in [
        (
            Command::ClusterReconnect {
                cluster: cluster.clone(),
            },
            CommandId::CLUSTER_RECONNECT,
            "app.cluster_reconnect",
        ),
        (
            Command::ClusterCancelConnect { cluster },
            CommandId::CLUSTER_CANCEL_CONNECT,
            "app.cluster_cancel_connect",
        ),
    ] {
        assert_eq!(command.id(), expected);
        let meta = command.meta();
        assert!(
            !meta.mutating && !meta.privileged,
            "{expected}: connecting reads, never writes"
        );
        assert_eq!(expected.tool_name(), tool);
    }
}

#[gpui::test]
fn open_terminal_is_disabled_until_the_host_has_a_terminal(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    auth_required(&mut fx, "prod-eu");
    assert!(
        fx.drawn("connect-open-terminal"),
        "visible, so the user knows it is coming"
    );
    assert!(
        fx.drawn("connect-terminal-unavailable"),
        "and says why it does nothing"
    );
    fx.click("connect-open-terminal");
    assert!(fx.terminals.borrow().is_empty());
    assert!(
        fx.recorder.sent().is_empty(),
        "a disabled button sends nothing"
    );
}

#[gpui::test]
fn open_terminal_runs_the_host_hook_for_this_cluster(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(
        cx,
        &["prod-eu", "dev-local"],
        Dispatch::Record,
        Offers {
            terminal: true,
            sources: false,
        },
    );
    auth_required(&mut fx, "prod-eu");
    assert!(!fx.drawn("connect-terminal-unavailable"));
    fx.click("connect-open-terminal");
    assert_eq!(*fx.terminals.borrow(), [id("prod-eu")]);
}

#[gpui::test]
fn edit_kubeconfig_sources_runs_the_host_hook(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(
        cx,
        &["prod-eu"],
        Dispatch::Record,
        Offers {
            terminal: false,
            sources: true,
        },
    );
    failed(&mut fx, "prod-eu");
    fx.click("connect-edit-sources");
    assert_eq!(fx.sources_opened.get(), 1);
}

#[gpui::test]
fn copy_details_puts_the_whole_redacted_text_on_the_clipboard(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["prod-eu"]);
    let long = format!("first line\n{}\nlast line", "x".repeat(1000));
    fx.connector
        .script()
        .connect
        .push_err(OxiError::validation(long.clone()));
    fx.connect("prod-eu");
    fx.click("connect-copy");
    let copied = fx
        .vcx
        .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .expect("something was copied");
    assert!(copied.contains("cluster: prod-eu"), "{copied}");
    assert!(
        copied.contains("server: https://prod-eu.example:6443"),
        "{copied}"
    );
    assert!(copied.contains(&long), "the full text, not the summary");
}

#[gpui::test]
fn a_new_state_collapses_the_details(cx: &mut TestAppContext) {
    let mut fx = Fixture::start(cx, &["prod-eu"], Dispatch::Run, Offers::default());
    failed(&mut fx, "prod-eu");
    fx.click("connect-details-toggle");
    let view = fx.view("prod-eu");
    assert!(fx.vcx.update(|_, cx| view.read(cx).details_open()));
    fx.connector
        .connect_script_for(&id("prod-eu"))
        .push_err(OxiError::validation("a different failure"));
    fx.click("connect-retry");
    assert_eq!(fx.phase("prod-eu"), SessionPhase::Error);
    assert!(
        !fx.drawn("connect-details"),
        "the new error starts with its summary"
    );
}
