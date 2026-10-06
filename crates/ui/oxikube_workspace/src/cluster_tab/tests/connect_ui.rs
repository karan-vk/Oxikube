//! A tab with connect views shows them by the session's phase: the body instead of the content
//! while connecting, needing credentials or failed; the banner above the content when degraded.

use gpui::{
    AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Styled as _, TestAppContext, Window, div,
};
use oxikube_domain::OxiError;
use oxikube_ports::HealthSignal;

use super::*;
use crate::cluster_tab::ConnectUi;

/// A view that is just a tagged box.
struct Marker(&'static str);

impl Render for Marker {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let selector = self.0;
        div()
            .debug_selector(move || selector.to_owned())
            .h(gpui::px(32.))
            .w_full()
            .child(selector)
    }
}

fn open(cx: &mut TestAppContext) -> Fixture {
    let mut fx = Fixture::plain(cx, &["alpha"]);
    fx.connector
        .connect_script_for(&id("alpha"))
        .push_err(OxiError::auth("token expired", false));
    // The first attempt needs credentials: the tab exists, with no views yet.
    block_on(fx.sessions.connect(&id("alpha"))).expect("connect");
    fx.vcx.run_until_parked();
    let tab = fx.tab("alpha");
    fx.vcx.update(|_, cx| {
        let body = cx.new(|_| Marker("marker-body"));
        let banner = cx.new(|_| Marker("marker-banner"));
        tab.update(cx, |tab, cx| {
            tab.set_connect_ui(
                ConnectUi {
                    body: body.into(),
                    banner: banner.into(),
                },
                cx,
            )
        });
    });
    fx.vcx.run_until_parked();
    fx
}

fn drawn(fx: &mut Fixture, selector: &'static str) -> bool {
    fx.vcx.update(|window, cx| window.draw(cx).clear(cx));
    fx.vcx.debug_bounds(selector).is_some()
}

#[gpui::test]
fn the_body_replaces_the_content_until_the_cluster_is_connected(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    assert!(drawn(&mut fx, "marker-body"), "AuthRequired shows the body");
    assert!(!drawn(&mut fx, "marker-banner"));
    assert!(!drawn(&mut fx, "cluster-placeholder-alpha"));

    block_on(fx.sessions.reconnect(&id("alpha"))).expect("reconnect");
    fx.vcx.run_until_parked();
    assert!(!drawn(&mut fx, "marker-body"), "Ready shows the cluster");
    assert!(!drawn(&mut fx, "marker-banner"));
    assert!(drawn(&mut fx, "cluster-placeholder-alpha"));
}

#[gpui::test]
fn the_banner_sits_above_the_content_while_degraded(cx: &mut TestAppContext) {
    let mut fx = open(cx);
    block_on(fx.sessions.reconnect(&id("alpha"))).expect("reconnect");
    fx.connector.report(&id("alpha"), HealthSignal::Unhealthy);
    fx.vcx.run_until_parked();

    assert!(!drawn(&mut fx, "marker-body"), "the content stays");
    assert!(drawn(&mut fx, "cluster-placeholder-alpha"));
    let banner = fx
        .vcx
        .debug_bounds("marker-banner")
        .expect("the banner is drawn");
    let content = fx
        .vcx
        .debug_bounds("cluster-placeholder-alpha")
        .expect("content");
    assert!(banner.bottom() <= content.top() + gpui::px(1.));
}

#[gpui::test]
fn without_connect_views_the_tab_keeps_its_placeholder(cx: &mut TestAppContext) {
    let mut fx = Fixture::plain(cx, &["alpha"]);
    fx.connect("alpha");
    assert!(
        fx.tab("alpha")
            .read_with(&fx.vcx, |tab, _| tab.connect_ui().is_none())
    );
    assert!(drawn(&mut fx, "cluster-placeholder-alpha"));
}
