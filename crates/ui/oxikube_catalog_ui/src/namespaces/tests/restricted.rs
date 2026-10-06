//! A cluster that refuses to list namespaces (RBAC), and names that no longer exist.

use gpui::TestAppContext;
use oxikube_app::session::namespaces::NamespaceSource;
use oxikube_domain::OxiError;
use oxikube_domain::session::NamespaceSelection;

use super::*;
use crate::namespaces::{NamespaceSelectorEvent, Row, stale_dropped_toast};

fn forbid_listing(env: &Env, times: usize) {
    for _ in 0..times {
        env.ports
            .resources
            .script()
            .list_metadata
            .push_err(OxiError::forbidden(
                "namespaces is forbidden: User cannot list resource \"namespaces\"",
            ));
    }
}

#[gpui::test]
fn a_403_on_the_namespace_list_shows_the_typed_names_fallback(cx: &mut TestAppContext) {
    let env = Env::new(&[]);
    env.remember(&NamespacePrefs {
        typed: vec!["team-a".into()],
        ..prefs(&[], &[])
    });
    forbid_listing(&env, 2);
    let mut window = open(cx, &env);
    window.run_until_parked();

    assert_eq!(
        window.read_root(|s, _| s.catalog().source),
        NamespaceSource::Forbidden
    );
    assert!(window.read_root(|s, _| s.is_restricted()));

    forbid_listing(&env, 1);
    open_dropdown(&mut window);

    let rows = window.read_root(|s, _| s.rows().to_vec());
    assert!(
        rows.iter().any(|r| r.namespace() == Some("team-a")),
        "{rows:?}"
    );
    assert!(
        window.bounds("namespace-notice").is_some(),
        "the fallback is explained"
    );
    let notice = window.read_root(|s, _| s.catalog().names.clone());
    assert_eq!(notice, ["team-a"]);
}

#[gpui::test]
fn typing_a_name_on_a_restricted_cluster_adds_and_selects_it(cx: &mut TestAppContext) {
    let env = Env::new(&[]);
    forbid_listing(&env, 3);
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);

    window.simulate_input("team-b");
    let rows = window.read_root(|s, _| s.rows().to_vec());
    assert_eq!(
        rows,
        [Row::Add {
            name: "team-b".into()
        }]
    );
    assert!(window.bounds("namespace-row-add").is_some());

    window.simulate_keystrokes("enter");
    env.settle(&window);

    assert_eq!(
        env.session_selection(),
        NamespaceSelection::single("team-b")
    );
    assert_eq!(
        env.stored().typed,
        ["team-b"],
        "remembered for the next run"
    );
    assert_eq!(window.read_root(|s, _| s.label()), "team-b");
}

#[gpui::test]
fn a_stored_namespace_that_no_longer_exists_is_dropped_with_an_event(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod"]);
    env.remember(&prefs(&["prod", "gone"], &[]));
    init_app(cx);
    let (service, cluster) = (env.service.clone(), env.cluster.clone());
    // Subscribe before the first task runs: the load starts when the window settles.
    let window = cx.add_window(|window, cx| NamespaceSelector::new(cluster, service, window, cx));
    let selector = window.entity(cx).expect("window is open");
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sink = events.clone();
    cx.update(|cx| {
        cx.subscribe(&selector, move |_, event: &NamespaceSelectorEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach();
    });
    cx.run_until_parked();

    assert_eq!(
        *events.borrow(),
        [NamespaceSelectorEvent::StaleDropped(vec![
            "gone".to_owned()
        ])]
    );
    assert_eq!(env.session_selection(), NamespaceSelection::single("prod"));
    assert_eq!(env.stored().selection, NamespaceSelection::single("prod"));
    cx.update(|cx| assert_eq!(selector.read(cx).label(), "prod"));
}

#[gpui::test]
fn the_host_shows_the_stale_name_toast(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = oxikube_workspace::test_support::open_workspace(cx);
    vcx.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            ws.show_toast(stale_dropped_toast(&["gone".to_owned()]), cx);
        })
    });
    vcx.run_until_parked();

    let layer = vcx.update(|_, cx| workspace.read(cx).toast_layer().clone());
    let visible = vcx.update(|_, cx| layer.read(cx).visible());
    assert_eq!(visible.len(), 1);
    assert_eq!(
        visible[0].message.as_ref(),
        "Namespace \"gone\" no longer exists and was removed from the selection."
    );

    // Several names, same key: the toast is replaced in place, not stacked.
    vcx.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            ws.show_toast(stale_dropped_toast(&["a".to_owned(), "b".to_owned()]), cx);
        })
    });
    let visible = vcx.update(|_, cx| layer.read(cx).visible());
    assert_eq!(visible.len(), 1);
    assert_eq!(
        visible[0].message.as_ref(),
        "Namespaces \"a\", \"b\" no longer exist and were removed from the selection."
    );
}
