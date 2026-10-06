//! The dropdown with the mouse: All, favourites, search, multi-select, the star.

use gpui::TestAppContext;
use oxikube_domain::session::NamespaceSelection;

use super::*;
use crate::namespaces::model::Row;

#[gpui::test]
fn the_dropdown_shows_all_the_favourites_and_the_namespaces(cx: &mut TestAppContext) {
    let env = Env::new(&["default", "dev", "prod"]);
    env.remember(&prefs(&[], &["prod"]));
    let mut window = open(cx, &env);
    window.run_until_parked();

    assert_eq!(window.read_root(|s, _| s.label()), "All namespaces");
    assert!(window.bounds("namespace-trigger").is_some());
    assert!(
        window.bounds("namespace-dropdown").is_none(),
        "closed at first"
    );

    open_dropdown(&mut window);

    let rows = window.read_root(|s, _| s.rows().to_vec());
    assert!(matches!(rows[0], Row::All { checked: true }));
    assert!(matches!(&rows[1], Row::Header(t) if t.as_ref() == "Favourites"));
    assert_eq!(rows[2].namespace(), Some("prod"));
    assert!(matches!(&rows[3], Row::Header(t) if t.as_ref() == "Namespaces"));
    assert_eq!(rows[4].namespace(), Some("default"));
    assert_eq!(rows[5].namespace(), Some("dev"));
    for visible in [
        "namespace-dropdown",
        "namespace-search",
        "namespace-row-all",
        "namespace-row-prod",
        "namespace-row-dev",
        "namespace-star-prod",
    ] {
        assert!(window.bounds(visible).is_some(), "{visible} is drawn");
    }
}

#[gpui::test]
fn typing_narrows_the_list_locally(cx: &mut TestAppContext) {
    let env = Env::new(&["default", "dev", "kube-system", "prod"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);
    let calls_before = env.ports.resources.recorded_calls().len();

    window.simulate_input("pro");
    window.draw_frame();

    let rows = window.read_root(|s, _| s.rows().to_vec());
    let names: Vec<_> = rows.iter().filter_map(Row::namespace).collect();
    assert_eq!(names, ["prod"]);
    assert!(window.bounds("namespace-row-prod").is_some());
    assert!(window.bounds("namespace-row-dev").is_none());
    assert_eq!(
        env.ports.resources.recorded_calls().len(),
        calls_before,
        "filtering never asks the cluster"
    );
}

#[gpui::test]
fn ticking_several_namespaces_changes_the_session_once_after_the_debounce(cx: &mut TestAppContext) {
    let env = Env::new(&["a", "b", "c"]);
    let mut updates = env.manager.subscribe();
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);

    click(&mut window, "namespace-row-a");
    click(&mut window, "namespace-row-b");
    click(&mut window, "namespace-row-c");

    // The view answers at once; the session waits for the quiet time.
    assert_eq!(
        window.read_root(|s, _| s.selection().clone()),
        NamespaceSelection::from_names(["a", "b", "c"])
    );
    assert_eq!(window.read_root(|s, _| s.label()), "a +2");
    assert_eq!(env.session_selection(), NamespaceSelection::All);

    env.settle(&window);

    assert_eq!(
        env.session_selection(),
        NamespaceSelection::from_names(["a", "b", "c"])
    );
    assert_eq!(
        namespace_changes(&mut updates).len(),
        1,
        "one re-scope for three ticks"
    );
    assert_eq!(
        env.stored().selection,
        env.session_selection(),
        "and it is remembered"
    );
}

#[gpui::test]
fn unticking_the_last_namespace_gives_all(cx: &mut TestAppContext) {
    let env = Env::new(&["a", "b"]);
    env.remember(&prefs(&["a"], &[]));
    let mut window = open(cx, &env);
    window.run_until_parked();
    assert_eq!(
        env.session_selection(),
        NamespaceSelection::single("a"),
        "restored"
    );
    open_dropdown(&mut window);

    click(&mut window, "namespace-row-a");
    env.settle(&window);

    assert_eq!(window.read_root(|s, _| s.label()), "All namespaces");
    assert_eq!(env.session_selection(), NamespaceSelection::All);
}

#[gpui::test]
fn the_all_row_clears_the_selection_at_once(cx: &mut TestAppContext) {
    let env = Env::new(&["a", "b"]);
    env.remember(&prefs(&["a", "b"], &[]));
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);

    click(&mut window, "namespace-row-all");

    assert_eq!(
        env.session_selection(),
        NamespaceSelection::All,
        "no debounce for All"
    );
    assert_eq!(window.read_root(|s, _| s.label()), "All namespaces");
}

#[gpui::test]
fn the_star_pins_a_namespace_and_remembers_it(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);

    click(&mut window, "namespace-star-prod");

    let (favourites, selection) = window.read_root(|s, _| {
        (
            s.favourites().iter().map(str::to_owned).collect::<Vec<_>>(),
            s.selection().clone(),
        )
    });
    assert_eq!(favourites, ["prod"]);
    assert_eq!(
        selection,
        NamespaceSelection::All,
        "the star does not tick the row"
    );
    assert_eq!(env.stored().favourites.iter().collect::<Vec<_>>(), ["prod"]);
    // The pinned namespace moves to the favourites section with digit 1.
    let first_fav = window.read_root(|s, _| s.rows()[2].clone());
    assert!(matches!(first_fav, Row::Namespace(r) if r.name == "prod" && r.slot == Some(1)));

    click(&mut window, "namespace-star-prod");
    assert!(window.read_root(|s, _| s.favourites().is_empty()));
    assert!(
        env.stored().favourites.is_empty(),
        "and unpinning is remembered"
    );
}

#[gpui::test]
fn the_selection_is_remembered_across_restarts(cx: &mut TestAppContext) {
    let env = Env::new(&["dev", "prod"]);
    {
        let mut window = open(cx, &env);
        window.run_until_parked();
        open_dropdown(&mut window);
        click(&mut window, "namespace-row-prod");
        env.settle(&window);
    }

    // A new run: the session starts with All and the service has no cache; the selector
    // restores what was stored.
    env.manager
        .set_namespace_selection(&env.cluster, NamespaceSelection::All)
        .unwrap();
    let fresh = NamespaceService::new(env.manager.clone(), env.state.clone(), env.clock.clone());
    let mut window = open_with(cx, &env, fresh);
    window.run_until_parked();

    assert_eq!(window.read_root(|s, _| s.label()), "prod");
    assert_eq!(env.session_selection(), NamespaceSelection::single("prod"));
}

#[gpui::test]
fn clicking_the_trigger_again_closes_the_dropdown(cx: &mut TestAppContext) {
    let env = Env::new(&["dev"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);

    click(&mut window, "namespace-trigger");

    assert!(
        !window.read_root(|s, _| s.is_open()),
        "the press must not close it and the click reopen it"
    );
    assert!(window.bounds("namespace-dropdown").is_none());
}

#[gpui::test]
fn clicking_outside_the_dropdown_and_trigger_closes_it(cx: &mut TestAppContext) {
    let env = Env::new(&["dev"]);
    let mut window = open(cx, &env);
    window.run_until_parked();
    open_dropdown(&mut window);
    let dropdown = window.bounds("namespace-dropdown").expect("drawn");

    window.simulate_click(
        point(
            dropdown.right() + gpui::px(40.),
            dropdown.bottom() + gpui::px(40.),
        ),
        gpui::Modifiers::none(),
    );
    window.draw_frame();

    assert!(!window.read_root(|s, _| s.is_open()));
}
