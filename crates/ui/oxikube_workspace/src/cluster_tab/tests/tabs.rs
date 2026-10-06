//! One tab per live session, each hosting a workspace of its own.

use futures::FutureExt as _;
use gpui::TestAppContext;
use oxikube_app::session::SessionOptions;
use oxikube_domain::ClusterColour;
use oxikube_domain::session::SessionPhase;

use super::*;
use crate::{
    DockPosition, Item as _,
    cluster_tab::{ClusterTabEvent, ClusterTabsEvent},
};

#[gpui::test]
fn two_connected_clusters_get_two_tabs_with_their_own_sidebar_and_panes(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    assert_eq!(
        fx.open_names(),
        Vec::<String>::new(),
        "nothing is connected yet"
    );

    fx.connect("alpha");
    fx.connect("beta");
    assert_eq!(fx.open_names(), ["alpha", "beta"]);
    assert_eq!(
        fx.active_name().as_deref(),
        Some("beta"),
        "a new tab is shown"
    );

    let (alpha, beta) = (fx.inner("alpha"), fx.inner("beta"));
    assert_ne!(alpha.entity_id(), beta.entity_id());
    // Each has its own sidebar: one left panel, titled after its own cluster.
    let titles = |fx: &mut Fixture, ws: &Entity<Workspace>| -> Vec<String> {
        fx.vcx.update(|_, cx| {
            ws.read(cx)
                .panels(cx)
                .iter()
                .map(|p| p.title(cx).to_string())
                .collect()
        })
    };
    assert_eq!(titles(&mut fx, &alpha), ["sidebar-alpha"]);
    assert_eq!(titles(&mut fx, &beta), ["sidebar-beta"]);

    // And its own pane group: an item opened in alpha is in alpha alone.
    fx.vcx.update(|window, cx| {
        let item = TestItem::build("pods", cx);
        alpha.update(cx, |ws, cx| ws.open_item(item, window, cx));
    });
    fx.vcx.run_until_parked();
    let count = |fx: &mut Fixture, ws: &Entity<Workspace>| {
        fx.vcx.update(|_, cx| ws.read(cx).items().count())
    };
    assert_eq!((count(&mut fx, &alpha), count(&mut fx, &beta)), (1, 0));

    // And its own docks: closing alpha's sidebar leaves beta's alone.
    let open = |fx: &mut Fixture, ws: &Entity<Workspace>| {
        fx.vcx
            .update(|_, cx| ws.read(cx).dock(DockPosition::Left, cx))
            .expect("a left dock")
            .is_open()
    };
    assert!(open(&mut fx, &alpha) && open(&mut fx, &beta));
    fx.vcx.update(|window, cx| {
        alpha.update(cx, |ws, cx| ws.toggle_dock(DockPosition::Left, window, cx));
    });
    fx.vcx.run_until_parked();
    assert!(!open(&mut fx, &alpha));
    assert!(open(&mut fx, &beta), "beta's dock is its own");
}

#[gpui::test]
fn a_tab_lives_exactly_as_long_as_its_session_is_connected(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    let events = Rc::new(RefCell::new(Vec::new()));
    let log = events.clone();
    let _subscription = fx.vcx.update(|_, cx| {
        cx.subscribe(&fx.tabs, move |_, event: &ClusterTabsEvent, _| {
            log.borrow_mut().push(event.clone())
        })
    });

    fx.connect("alpha");
    fx.connect("beta");
    assert_eq!(fx.open_names(), ["alpha", "beta"]);

    // Disconnecting from anywhere (the catalog, an agent) closes the tab: the session is the
    // one authority.
    fx.disconnect("alpha");
    assert_eq!(fx.open_names(), ["beta"]);
    assert_eq!(
        fx.vcx.update(|_, cx| fx.ws.read(cx).items().count()),
        2,
        "the catalog and beta"
    );

    // Connecting again opens a fresh tab.
    fx.connect("alpha");
    assert_eq!(fx.open_names(), ["beta", "alpha"]);

    // Forgetting the session closes it too.
    assert!(fx.sessions.close(&id("beta")));
    fx.vcx.run_until_parked();
    assert_eq!(fx.open_names(), ["alpha"]);

    let events = events.borrow();
    let opened = events
        .iter()
        .filter(|e| matches!(e, ClusterTabsEvent::Opened(_)))
        .count();
    let closed = events
        .iter()
        .filter(|e| matches!(e, ClusterTabsEvent::Closed(_)))
        .count();
    assert_eq!((opened, closed), (3, 2));
}

#[gpui::test]
fn a_connecting_session_already_has_its_tab(cx: &mut TestAppContext) {
    let mut fx = Fixture::plain(cx, &["alpha"]);
    fx.connector.hold();
    let (sessions, cluster) = (fx.sessions.clone(), id("alpha"));
    let mut connect = Box::pin(sessions.connect(&cluster));
    assert!(
        connect.as_mut().now_or_never().is_none(),
        "held at the connector"
    );
    fx.vcx.run_until_parked();
    assert_eq!(
        fx.open_names(),
        ["alpha"],
        "the tab is where Connecting shows"
    );
    let phase = fx.sessions.get(&id("alpha")).map(|s| s.phase());
    assert_eq!(phase, Some(SessionPhase::Connecting));

    // The placeholder says what the session is doing.
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        fx.vcx.debug_bounds("cluster-placeholder-alpha").is_some(),
        "an empty cluster workspace shows its placeholder"
    );

    fx.connector.release();
    block_on(connect).expect("connect");
    fx.vcx.run_until_parked();
    assert_eq!(fx.open_names(), ["alpha"]);
}

#[gpui::test]
fn the_tab_shows_the_session_colour_and_name(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    let red = ClusterColour::rgb(0xe5, 0x39, 0x35);
    // Opened with a colour and a name before it connects, as a cluster with settings is.
    fx.sessions.open(
        &context("alpha"),
        SessionOptions {
            colour: Some(red),
            display_name: Some("Production".into()),
            ..Default::default()
        },
    );
    fx.connect("alpha");
    fx.connect("beta");

    let (alpha, beta) = (fx.tab("alpha"), fx.tab("beta"));
    let content = |fx: &mut Fixture, tab: &Entity<ClusterTab>| {
        fx.vcx.update(|_, cx| tab.read(cx).tab_content(cx))
    };
    let alpha_content = content(&mut fx, &alpha);
    assert_eq!(alpha_content.title, "Production");
    assert_eq!(colour_of(&alpha_content), Some(red));
    assert_eq!(colour_of(&content(&mut fx, &beta)), None, "no colour set");

    // A colour set later reaches the tab at once.
    let blue = ClusterColour::rgb(0x1e, 0x88, 0xe5);
    fx.set_colour("beta", Some(blue));
    assert_eq!(colour_of(&content(&mut fx, &beta)), Some(blue));
    fx.set_colour("beta", None);
    assert_eq!(colour_of(&content(&mut fx, &beta)), None);
}

#[gpui::test]
fn the_tab_shows_the_read_only_lock_of_its_session(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    fx.connect("alpha");
    let tab = fx.tab("alpha");
    let read_only = |fx: &mut Fixture| {
        fx.vcx
            .update(|_, cx| tab.read(cx).tab_content(cx))
            .cluster
            .is_some_and(|mark| mark.read_only)
    };
    assert!(!read_only(&mut fx));
    fx.sessions
        .set_read_only(&id("alpha"), true)
        .expect("open session");
    fx.vcx.run_until_parked();
    assert!(read_only(&mut fx), "read-only reaches the tab at once");
    // A colour change keeps the lock.
    fx.set_colour("alpha", Some(ClusterColour::rgb(1, 2, 3)));
    assert!(read_only(&mut fx));
}

fn colour_of(content: &crate::item::TabContent) -> Option<ClusterColour> {
    content.cluster.and_then(|mark| mark.colour)
}

#[gpui::test]
fn a_hidden_tab_is_told_it_is_hidden(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    let (alpha, beta) = (fx.tab("alpha"), fx.tab("beta"));
    let is_active = |fx: &mut Fixture, tab: &Entity<ClusterTab>| {
        fx.vcx.update(|_, cx| tab.read(cx).is_active())
    };
    assert!(!is_active(&mut fx, &alpha), "alpha is behind beta");
    assert!(is_active(&mut fx, &beta));

    let events = Rc::new(RefCell::new(Vec::new()));
    let log = events.clone();
    let _subscription = fx.vcx.update(|_, cx| {
        cx.subscribe(&alpha, move |_, event: &ClusterTabEvent, _| {
            log.borrow_mut().push(*event)
        })
    });
    assert!(fx.apply(Command::ClusterSelect {
        cluster: id("alpha")
    }));
    assert!(is_active(&mut fx, &alpha) && !is_active(&mut fx, &beta));
    assert_eq!(*events.borrow(), [ClusterTabEvent::ActiveChanged(true)]);

    // Showing the catalog hides every cluster: nobody is active, so nothing polls.
    let home = fx.vcx.update(|_, cx| {
        fx.ws
            .read(cx)
            .items()
            .find(|item| item.tab_content(cx).title == "Clusters")
            .map(|item| item.item_id())
    });
    fx.vcx.update(|window, cx| {
        fx.ws.update(cx, |ws, cx| {
            ws.activate_item(home.expect("catalog"), true, window, cx)
        })
    });
    fx.vcx.run_until_parked();
    assert_eq!(fx.active_name(), None);
    assert!(!is_active(&mut fx, &alpha));
}

#[gpui::test]
fn only_the_displayed_cluster_is_drawn(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha", "beta"]);
    fx.connect("alpha");
    fx.connect("beta");
    for (name, title) in [("alpha", "pods-a"), ("beta", "pods-b")] {
        let inner = fx.inner(name);
        fx.vcx.update(|window, cx| {
            let item = TestItem::build(title, cx);
            inner.update(cx, |ws, cx| ws.open_item(item, window, cx));
        });
    }
    fx.vcx.run_until_parked();
    let drawn = |fx: &mut Fixture, selector: &'static str| {
        fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
        fx.vcx.debug_bounds(selector).is_some()
    };

    // beta is displayed: alpha's views are not even built into the frame.
    assert!(drawn(&mut fx, "item-pods-b"));
    assert!(
        !drawn(&mut fx, "item-pods-a"),
        "a hidden cluster does not render"
    );

    assert!(fx.apply(Command::ClusterSelect {
        cluster: id("alpha")
    }));
    assert!(drawn(&mut fx, "item-pods-a"));
    assert!(!drawn(&mut fx, "item-pods-b"));
}

#[gpui::test]
fn the_tab_label_has_the_cluster_dot_and_a_close_button(cx: &mut TestAppContext) {
    let mut fx = Fixture::open(cx, &["alpha"]);
    fx.sessions.open(
        &context("alpha"),
        SessionOptions {
            colour: Some(ClusterColour::rgb(200, 30, 30)),
            ..Default::default()
        },
    );
    fx.connect("alpha");
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(fx.vcx.debug_bounds("tab-alpha").is_some(), "the tab label");
    assert!(
        fx.vcx.debug_bounds("tab-close-alpha").is_some(),
        "its own close button"
    );
    assert!(
        fx.vcx.debug_bounds("cluster-stripe-alpha").is_some(),
        "and the stripe over the body"
    );
}
