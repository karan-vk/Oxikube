//! Immediate commands through the runner (E05-P600): `namespace::Select` changes the session in
//! the dispatching update and the views observing the [`SessionEcho`] hear it in that update; the
//! remembering runs afterwards, off the UI thread.

use futures::executor::block_on;
use gpui::{AppContext as _, Context, Entity, Subscription, TestAppContext};
use oxikube_app::SessionChange;
use oxikube_app::session::namespaces::prefs_key;
use oxikube_domain::command::Command;
use oxikube_domain::session::NamespaceSelection;
use oxikube_ports::StatePort as _;

use super::{Fixture, fixture};
use crate::cluster::tests::fixture::{LAB, PROD, id};
use crate::cluster::{EchoItem, SessionEcho, observe_session_echo};
use crate::toast::ToastLevel;

/// Records what the echo hands it.
struct Listener {
    heard: Vec<EchoItem>,
    calls: usize,
    _echo: Subscription,
}

fn listener(f: &mut Fixture) -> Entity<Listener> {
    f.vcx.update(|_, cx| {
        cx.new(|cx: &mut Context<Listener>| Listener {
            heard: Vec::new(),
            calls: 0,
            _echo: observe_session_echo(cx, |this: &mut Listener, items, _| {
                this.calls += 1;
                this.heard.extend(items.iter().cloned());
            }),
        })
    })
}

fn heard_namespaces(f: &mut Fixture, listener: &Entity<Listener>) -> Vec<NamespaceSelection> {
    f.vcx.update(|_, cx| {
        listener
            .read(cx)
            .heard
            .iter()
            .filter_map(|item| match item {
                Ok(update) if update.cluster == id(PROD) => match &update.change {
                    SessionChange::NamespaceChanged(selection) => Some(selection.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    })
}

fn select(cluster: &str, names: &[&str]) -> Command {
    Command::NamespaceSelect {
        cluster: id(cluster),
        namespaces: names.iter().map(|n| (*n).to_owned()).collect(),
    }
}

fn selection(f: &Fixture) -> NamespaceSelection {
    f.manager
        .get(&id(PROD))
        .unwrap()
        .namespace_selection()
        .clone()
}

#[gpui::test]
fn namespace_select_lands_in_the_dispatching_update_and_is_remembered_after(
    cx: &mut TestAppContext,
) {
    let mut f = fixture(cx);
    let listener = listener(&mut f);
    let runner = f.runner.clone();

    // One update, no executor turn after it: what the frame drawn right after the input sees.
    f.vcx
        .update(|window, cx| runner.run(select(PROD, &["web"]), window, cx));

    assert_eq!(selection(&f), NamespaceSelection::single("web"));
    assert_eq!(
        heard_namespaces(&mut f, &listener),
        [NamespaceSelection::single("web")],
        "the views heard the change in the same update"
    );
    let key = prefs_key(&id(PROD));
    assert_eq!(
        block_on(f.state.kv_get(&key)).unwrap(),
        None,
        "remembering is not on the UI thread's path"
    );

    f.vcx.run_until_parked();
    assert!(
        block_on(f.state.kv_get(&key)).unwrap().is_some(),
        "remembered once the rest ran"
    );
    let toasts = f
        .vcx
        .update(|_, cx| f.ws.read(cx).toast_layer().read(cx).visible().len());
    assert_eq!(toasts, 0, "a success shows nothing");
}

#[gpui::test]
fn a_selection_that_changes_nothing_echoes_nothing(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let runner = f.runner.clone();
    f.vcx
        .update(|window, cx| runner.run(select(PROD, &["web"]), window, cx));
    f.vcx.run_until_parked();
    let listener = listener(&mut f);

    f.vcx
        .update(|window, cx| runner.run(select(PROD, &["web"]), window, cx));

    assert_eq!(f.vcx.update(|_, cx| listener.read(cx).calls), 0);
}

#[gpui::test]
fn an_immediate_command_that_fails_shows_a_toast(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    f.manager.close(&id(LAB));
    let runner = f.runner.clone();

    f.vcx
        .update(|window, cx| runner.run(select(LAB, &["web"]), window, cx));
    f.vcx.run_until_parked();

    let shown: Vec<_> = f.vcx.update(|_, cx| {
        f.ws.read(cx)
            .toast_layer()
            .read(cx)
            .visible()
            .iter()
            .map(|t| t.level)
            .collect()
    });
    assert_eq!(shown, [ToastLevel::Error], "no session: an error toast");
}

#[gpui::test]
fn the_echo_carries_only_what_was_sent_between_begin_and_finish(cx: &mut TestAppContext) {
    let mut f = fixture(cx);
    let listener = listener(&mut f);
    let manager = f.manager.clone();
    manager
        .set_namespace_selection(&id(PROD), NamespaceSelection::single("before"))
        .unwrap();

    f.vcx.update(|_, cx| {
        let echo = SessionEcho::begin(&manager);
        manager
            .set_namespace_selection(&id(PROD), NamespaceSelection::single("during"))
            .unwrap();
        echo.finish(cx);
    });
    assert_eq!(
        heard_namespaces(&mut f, &listener),
        [NamespaceSelection::single("during")]
    );

    // Nothing sent: the views are not called.
    f.vcx
        .update(|_, cx| SessionEcho::begin(&manager).finish(cx));
    assert_eq!(f.vcx.update(|_, cx| listener.read(cx).calls), 1);
}
