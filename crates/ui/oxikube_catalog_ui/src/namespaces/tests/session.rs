//! The selector follows the session and goes through the service.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::session::NamespaceSelection;

use super::*;

#[gpui::test]
fn a_selection_changed_by_a_command_shows_in_the_trigger(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    assert_eq!(window.read_root(|s, _| s.label()), "All namespaces");

    // The palette or an agent runs the same command the selector runs.
    futures::executor::block_on(env.service.execute(&Command::NamespaceSelect {
        cluster: env.cluster.clone(),
        namespaces: vec!["prod".into(), "dev".into()],
    }))
    .unwrap();
    window.run_until_parked();

    assert_eq!(window.read_root(|s, _| s.label()), "dev +1");
    assert_eq!(
        window.read_root(|s, _| s.selection().clone()),
        NamespaceSelection::from_names(["dev", "prod"])
    );
}

#[gpui::test]
fn a_command_during_a_pending_tick_wins_and_the_view_follows(cx: &mut TestAppContext) {
    let env = Env::new(&["a", "b"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);
    click(&mut window, "namespace-row-a");
    assert_eq!(window.read_root(|s, _| s.label()), "a");

    // An agent selects `b` while the tick of `a` is still waiting for its quiet time.
    futures::executor::block_on(env.service.execute(&Command::NamespaceSelect {
        cluster: env.cluster.clone(),
        namespaces: vec!["b".into()],
    }))
    .unwrap();
    env.settle(&window);

    assert_eq!(env.session_selection(), NamespaceSelection::single("b"));
    assert_eq!(
        window.read_root(|s, _| s.selection().clone()),
        env.session_selection(),
        "the view shows what the feeds are scoped to"
    );
    assert_eq!(window.read_root(|s, _| s.label()), "b");
}

#[gpui::test]
fn a_failed_write_is_reported_and_the_view_goes_back(cx: &mut TestAppContext) {
    let env = Env::new(&["dev"]);
    env.remember(&prefs(&[], &["dev"]));
    let mut window = open(cx, &env);
    window.run_until_parked();
    window.update_root(|s, window, cx| window.focus(&s.trigger_focus.clone(), cx));
    env.state
        .script()
        .kv_set
        .push_err(oxikube_domain::OxiError::internal("disk full"));

    window.simulate_keystrokes("1");
    window.run_until_parked();

    let error = window.read_root(|s, _| s.error().cloned());
    assert_eq!(error.as_deref(), Some("disk full"));
    // The session did change (the write is what failed); the view shows the session's truth.
    assert_eq!(
        window.read_root(|s, _| s.selection().clone()),
        env.session_selection()
    );
}

#[gpui::test]
fn the_selector_never_touches_the_cluster_except_to_list_namespaces(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);
    click(&mut window, "namespace-row-dev");
    env.settle(&window);

    let calls = env.ports.resources.recorded_calls();
    assert!(!calls.is_empty(), "it listed the namespaces");
    assert!(
        calls.iter().all(|c| !c.is_mutating()),
        "no cluster mutation: {calls:?}"
    );
}

#[test]
fn the_selector_actions_are_not_named_like_commands() {
    use gpui::Action as _;
    // `namespace::*` names belong to the commands (the keymap, the palette and the tools use
    // them); the view's own actions live in `namespace_selector::*`.
    for name in [
        crate::namespaces::Open.name(),
        crate::namespaces::Close.name(),
        crate::namespaces::SelectSlot { slot: 1 }.name(),
        crate::namespaces::ToggleHighlighted.name(),
    ] {
        assert!(name.starts_with("namespace_selector::"), "{name}");
        assert!(
            oxikube_domain::command::lookup_str(name).is_none(),
            "{name}"
        );
    }
}

#[gpui::test]
fn a_digit_changes_the_session_in_the_key_update_and_remembers_it_after(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod"]);
    env.remember(&prefs(&[], &["prod"]));
    let mut window = open(cx, &env);
    window.run_until_parked();

    // E05-P600: no executor turn between the key and the check, as for the frame after the key.
    let root = window.root();
    window.update(|_, cx| root.update(cx, |s, cx| s.select_slot(1, cx)));
    assert_eq!(env.session_selection(), NamespaceSelection::single("prod"));
    assert_eq!(window.read_root(|s, _| s.label()), "prod");

    window.run_until_parked();
    assert_eq!(env.stored().selection, NamespaceSelection::single("prod"));
}

#[gpui::test]
fn a_selection_echoed_by_a_command_on_the_ui_thread_shows_in_that_update(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod"]);
    let mut window = open(cx, &env);
    window.run_until_parked();

    // What the command runner does for `namespace::Select` (E05-P600).
    let (service, cluster) = (env.service.clone(), env.cluster.clone());
    window.update(move |_, cx| {
        let echo = oxikube_workspace::cluster::SessionEcho::begin(service.manager());
        let selected = service
            .select_now(&cluster, NamespaceSelection::single("dev"))
            .unwrap();
        echo.finish(cx);
        drop(selected.remember());
    });

    assert_eq!(window.read_root(|s, _| s.label()), "dev");
}
