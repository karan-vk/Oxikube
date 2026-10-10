//! `[`, `]` and `-` in a table, and `:-` from the bar: the session's lines run again.

use gpui::TestAppContext;
use oxikube_app::search::jump::HistoryStep;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;

use super::{Fixture, id, wait_for_data};
use crate::jump::JumpRequest;

fn open(group: &str, kind: &str) -> Command {
    Command::ResourceOpenList {
        cluster: id("dev"),
        gvk: Gvk::new(group, "v1", kind),
    }
}

fn deploy_web() -> Vec<Command> {
    vec![
        Command::NamespaceSelect {
            cluster: id("dev"),
            namespaces: vec!["web".to_owned()],
        },
        open("apps", "Deployment"),
    ]
}

/// Runs `pods`, `deploy web` and `ns`, and forgets what they sent.
fn three_lines(f: &mut Fixture) {
    f.open();
    wait_for_data(f);
    f.keys("escape");
    for line in ["pods", "deploy web", "ns"] {
        f.run_line(line);
    }
    let _ = f.take_sent();
}

#[gpui::test]
fn the_history_keeps_the_canonical_lines_that_were_run(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.run_line("deploy /api web");
    f.run_line("q");
    f.run_line("-");
    assert_eq!(f.host.history(&id("dev")).lines(), ["deploy web /api"]);
}

#[gpui::test]
fn the_bracket_keys_step_back_and_forward_through_the_lines(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    three_lines(&mut f);

    f.keys("[");
    assert_eq!(f.take_sent(), deploy_web(), "`[` runs the line before `ns`");
    f.keys("[");
    assert_eq!(f.take_sent(), [open("", "Pod")]);
    f.keys("[");
    assert_eq!(f.take_sent(), [], "the oldest line has nothing before it");
    f.keys("]");
    assert_eq!(f.take_sent(), deploy_web());
    f.keys("] ]");
    assert_eq!(f.take_sent(), [open("", "Namespace")], "the newest, once");
    assert_eq!(
        f.host.history(&id("dev")).len(),
        3,
        "stepping adds no lines"
    );
}

#[gpui::test]
fn the_dash_key_goes_to_the_previous_view_and_back(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    three_lines(&mut f);
    f.keys("-");
    assert_eq!(f.take_sent(), deploy_web());
    f.keys("-");
    assert_eq!(
        f.take_sent(),
        [open("", "Namespace")],
        "a second dash returns"
    );
}

#[gpui::test]
fn dash_with_nothing_before_it_says_so_and_sends_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.keys("-");
    assert_eq!(f.take_sent(), []);
    f.keys("[");
    assert_eq!(f.take_sent(), []);
}

#[gpui::test]
fn the_history_words_in_the_bar_send_the_history_commands(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    three_lines(&mut f);
    for (line, command) in [
        ("-", Command::JumpLast),
        ("[", Command::JumpBack),
        ("]", Command::JumpForward),
    ] {
        f.run_line(line);
        assert_eq!(f.take_sent(), [command], ":{line}");
    }
    assert_eq!(
        f.host.history(&id("dev")).len(),
        3,
        "a history word is not a line to come back to"
    );
}

#[gpui::test]
fn the_bus_requests_do_what_the_keys_do(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    three_lines(&mut f);
    let host = f.host.clone();
    f.vcx
        .update(|window, cx| host.apply(JumpRequest::Step(HistoryStep::Back), window, cx));
    assert_eq!(f.take_sent(), deploy_web());
}
