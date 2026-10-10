//! Typing a line and pressing Enter: the commands that go out, and the errors that stay.

use gpui::TestAppContext;
use oxikube_app::search::jump::ParseErrorKind;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;

use super::{Fixture, id, wait_for_data};

fn pods() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

#[gpui::test]
fn pods_opens_the_pods_list_and_closes_the_bar(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.run_line("pods");
    assert!(f.bar().is_none(), "a good line closes the bar");
    assert!(
        f.table_has_focus(),
        "and gives the focus back before the commands run"
    );
    assert_eq!(
        f.take_sent(),
        [Command::ResourceOpenList {
            cluster: id("dev"),
            gvk: pods()
        }]
    );
}

#[gpui::test]
fn deploy_kube_system_selects_the_namespace_then_opens_the_list(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    wait_for_data(&mut f);
    f.type_text("deploy kube-system");
    f.keys("enter");
    assert_eq!(
        f.take_sent(),
        [
            Command::NamespaceSelect {
                cluster: id("dev"),
                namespaces: vec!["kube-system".to_owned()],
            },
            Command::ResourceOpenList {
                cluster: id("dev"),
                gvk: Gvk::new("apps", "v1", "Deployment"),
            },
        ]
    );
}

#[gpui::test]
fn a_filter_and_a_selector_go_out_as_one_set_filter(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.run_line("pod /api app=x");
    assert_eq!(
        f.take_sent(),
        [
            Command::ResourceOpenList {
                cluster: id("dev"),
                gvk: pods()
            },
            Command::TableSetFilter {
                cluster: id("dev"),
                gvk: pods(),
                text: "api -l app=x".to_owned(),
            },
        ]
    );
}

#[gpui::test]
fn ctx_and_at_name_switch_to_a_connected_cluster_tab(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.run_line("ns");
    assert_eq!(
        f.take_sent(),
        [Command::ResourceOpenList {
            cluster: id("dev"),
            gvk: Gvk::new("", "v1", "Namespace"),
        }]
    );
    f.run_line("ctx dev");
    assert_eq!(
        f.take_sent(),
        [Command::ClusterSelect { cluster: id("dev") }]
    );
}

#[gpui::test]
fn q_sends_the_quit_command(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.run_line("q");
    assert_eq!(f.take_sent(), [Command::AppQuit]);
}

#[gpui::test]
fn an_unknown_alias_stays_in_the_bar_with_suggestions(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.run_line("podz");
    assert!(f.bar().is_some(), "the bar stays open on a mistake");
    let (kind, span, suggestions) = f.read(|d| {
        let e = d.error().expect("the error is kept");
        (e.kind, e.span, e.suggestions.clone())
    });
    assert_eq!(kind, ParseErrorKind::UnknownAlias);
    assert_eq!(&"podz"[span.range()], "podz");
    assert!(suggestions.iter().any(|s| &**s == "pods"));
    assert!(
        f.vcx.debug_bounds("jump-problem").is_some(),
        "the problem line is drawn"
    );
    assert_eq!(f.take_sent(), [], "nothing ran");

    // The next keystroke clears the error: the user is fixing it.
    f.type_text("s");
    assert!(f.read(|d| d.error().is_none()));
}

#[gpui::test]
fn an_unknown_namespace_is_an_error_once_the_cluster_list_is_known(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    wait_for_data(&mut f);
    f.type_text("pods kube-systm");
    f.keys("enter");
    assert!(f.bar().is_some());
    let (kind, first) = f.read(|d| {
        let e = d.error().expect("an error");
        (e.kind, e.suggestions.first().map(|s| s.to_string()))
    });
    assert_eq!(kind, ParseErrorKind::UnknownNamespace);
    assert_eq!(first.as_deref(), Some("kube-system"));
}

#[gpui::test]
fn the_syntax_error_so_far_is_shown_quietly_while_typing(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    f.type_text("pods @");
    assert_eq!(
        f.read(|d| d.syntax_error().map(|e| e.kind)),
        Some(ParseErrorKind::MissingContext)
    );
    assert!(f.read(|d| d.error().is_none()), "not an Enter error");
    assert!(f.vcx.debug_bounds("jump-problem").is_some());
    f.type_text("dev");
    assert!(f.read(|d| d.syntax_error().is_none()));
}

#[gpui::test]
fn enter_on_an_empty_line_just_closes_the_bar(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.open();
    f.keys("enter");
    assert!(f.bar().is_none());
    assert_eq!(f.take_sent(), []);
}
