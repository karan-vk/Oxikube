//! The sign-in view's "Open terminal" in the running app (E06-U558): it is enabled, it sends
//! `terminal::New` for the cluster, and the shell is visible under the sign-in view while the
//! cluster is not connected.

use std::rc::Rc;

use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_terminal::view::{BackendDescriptor, TerminalServices, TerminalView};
use oxikube_testkit::TestPorts;
use oxikube_workspace::DockPosition;

use super::App;
use super::terminal::FakeLauncher;

fn auth_required_with_fake_launcher(cx: &mut TestAppContext) -> (App, Rc<FakeLauncher>) {
    let ports = TestPorts::seeded();
    ports
        .connector
        .connect_script_for(&TestPorts::cluster_id())
        .push_err(OxiError::auth("token expired: run `aws sso login`", false));
    let launcher = Rc::new(FakeLauncher::default());
    let installed = launcher.clone();
    let mut app = App::start_with(cx, ports, move |cx| {
        oxikube_terminal::view::install(TerminalServices::new(installed), cx);
    });
    app.press("enter");
    (app, launcher)
}

#[gpui::test]
fn open_terminal_on_the_sign_in_view_opens_a_cluster_terminal_under_it(cx: &mut TestAppContext) {
    let (mut app, launcher) = auth_required_with_fake_launcher(cx);
    assert!(app.drawn("connect-auth"), "the sign-in view is shown");
    assert!(app.drawn("connect-open-terminal"));
    assert!(
        !app.drawn("connect-terminal-unavailable"),
        "the terminal exists now: no 'not available yet' note"
    );
    let terminal_slot = format!("cluster-connect-terminal-{}", TestPorts::CONTEXT);
    assert!(!app.drawn(&terminal_slot), "no terminal until asked");

    app.click("connect-open-terminal");

    // `terminal::New` for this cluster: a shell with its context, in the tab's bottom dock.
    let launches = launcher.launches.borrow().clone();
    assert_eq!(launches.len(), 1);
    assert!(matches!(&launches[0], BackendDescriptor::Local { .. }));
    assert_eq!(launches[0].cluster(), Some(&TestPorts::cluster_id()));
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    let (terminals, docked) = app.vcx.update(|_, cx| {
        let ws = tab.read(cx).workspace().read(cx);
        let terminals = ws.items_of_type::<TerminalView>();
        let docked = terminals
            .first()
            .and_then(|view| ws.item_dock(view.entity_id(), cx));
        (terminals.len(), docked)
    });
    assert_eq!(terminals, 1);
    assert_eq!(docked, Some(DockPosition::Bottom));

    // The cluster's own content is not drawn yet, so the tab shows the shell under the sign-in
    // view: the user can run the login and press Retry.
    assert!(app.drawn(&terminal_slot), "the terminal is visible");
    assert!(app.drawn("connect-auth"), "the sign-in view stays above it");
    assert!(app.drawn("connect-retry"));
}

#[gpui::test]
fn a_connected_tab_does_not_draw_the_sign_in_terminal_slot(cx: &mut TestAppContext) {
    let (mut app, _launcher) = auth_required_with_fake_launcher(cx);
    app.click("connect-open-terminal");
    let terminal_slot = format!("cluster-connect-terminal-{}", TestPorts::CONTEXT);
    assert!(
        app.drawn(&terminal_slot),
        "the terminal is under the sign-in view"
    );

    // The retry connects (the script is exhausted, so the fake succeeds): the tab hands the
    // screen to the workspace, and the sign-in slot (and its per-frame lookup) goes away.
    app.click("connect-retry");
    assert!(
        !app.drawn("connect-auth"),
        "the sign-in view is gone once connected"
    );
    assert!(
        !app.drawn(&terminal_slot),
        "a connected tab has no sign-in terminal slot"
    );
}
