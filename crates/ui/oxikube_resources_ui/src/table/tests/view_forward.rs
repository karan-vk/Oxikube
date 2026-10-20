//! The port-forward start dialog (E15-S04), opened by `shift-f` on a pod row and driven like a
//! user would: it opens at once (no I/O first), reads the pod's ports, validates as typed, sends
//! `portforward::Start` on submit, shows the URL toast with "Open in browser", keeps a failed start
//! open with its message and one-click fix, and is blocked on a read-only cluster unless
//! `portforward.allowed_in_readonly`.

use std::net::SocketAddr;

use gpui::TestAppContext;
use oxikube_app::{ForwardConfig, StartError};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::portforward::ForwardPort;
use oxikube_portforward_ui::{Fix, Stage, StartDialog};
use serde_json::json;

use super::fixture::{Fixture, cluster};
use super::forward_rig::Reply;
use super::p;
use crate::actions::tests::toasts;
use crate::detail::tests::fixture::edited;
use crate::table::ResourceTable;

pub(super) fn apple() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "x", "apple-1")
}

/// A pod `x/apple-1` with two container ports.
pub(super) fn apple_pod() -> oxikube_domain::Resource {
    edited(p("x", "apple-1", "1"), |json| {
        json["spec"]["containers"] = json!([
            {"name": "app", "image": "app:1", "ports": [
                {"name": "http", "containerPort": 8080},
                {"containerPort": 9090},
            ]}
        ]);
    })
}

pub(super) fn open(cx: &mut TestAppContext) -> (Fixture, gpui::Entity<ResourceTable>) {
    let mut f = Fixture::with_forward(cx);
    f.connect_with([apple_pod(), p("x", "web-1", "1")]);
    let table = f.open_pods();
    f.update(&table, |t, cx| t.move_cursor(1, false, cx));
    assert_eq!(f.selected(&table), ["apple-1"]);
    f.dispatcher.clear();
    (f, table)
}

pub(super) fn dialog(f: &mut Fixture) -> Option<gpui::Entity<StartDialog>> {
    let tabs = f.tabs.clone();
    f.vcx.update(|_, cx| {
        let tab = tabs.read(cx).tab(&cluster())?.clone();
        let workspace = tab.read(cx).workspace().clone();
        let layer = workspace.read(cx).modal_layer().clone();
        layer.read(cx).active_modal::<StartDialog>()
    })
}

pub(super) fn open_dialog(
    f: &mut Fixture,
    table: &gpui::Entity<ResourceTable>,
) -> gpui::Entity<StartDialog> {
    f.keys(table, "shift-f");
    dialog(f).expect("shift-f opens the start dialog")
}

fn messages(f: &mut Fixture) -> Vec<String> {
    toasts(f)
        .into_iter()
        .map(|t| t.message.to_string())
        .collect()
}

pub(super) fn click(f: &mut Fixture, selector: &'static str) {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let bounds = f
        .vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is on screen"));
    f.vcx.simulate_click(bounds.center(), Default::default());
    f.settle();
}

pub(super) fn shown(f: &mut Fixture, selector: &'static str) -> bool {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    f.vcx.debug_bounds(selector).is_some()
}

#[gpui::test]
fn shift_f_opens_the_dialog_on_the_pods_ports_and_sends_nothing(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    let d = open_dialog(&mut f, &table);
    f.settle();
    assert!(shown(&mut f, "portforward-start-dialog"));
    for field in [
        "pf-remote",
        "pf-local",
        "pf-auto",
        "pf-bind",
        "pf-name",
        "pf-favourite",
    ] {
        assert!(shown(&mut f, field), "{field}");
    }
    // The ports were read from the pod, and the first one filled the remote port.
    assert!(shown(&mut f, "pf-port-0") && shown(&mut f, "pf-port-1"));
    let input = f.vcx.update(|_, cx| d.read(cx).input(cx));
    assert_eq!(input.remote_port, "8080");
    assert!(input.local_auto);
    assert_eq!(input.bind, "127.0.0.1");
    assert!(f.forward.as_ref().unwrap().calls.lock().is_empty());
    assert!(f.dispatcher.sent().is_empty());
}

#[gpui::test]
fn submit_sends_portforward_start_with_every_field_and_toasts_the_url(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    let d = open_dialog(&mut f, &table);
    f.settle();
    f.vcx.update(|window, cx| {
        d.update(cx, |d, cx| {
            d.fill("9090", "18080", "127.0.0.1", window, cx);
            d.set_favourite(true, cx);
        });
    });
    f.settle();
    click(&mut f, "pf-submit");

    let starts = f.forward.as_ref().unwrap().starts();
    assert_eq!(
        starts,
        [Command::PortForwardStart {
            target: apple(),
            remote_port: ForwardPort::Number(9090),
            local_port: Some(18080),
            bind: Some("127.0.0.1".parse().unwrap()),
            name: None,
            favourite: true,
        }]
    );
    assert!(
        dialog(&mut f).is_none(),
        "a started forward closes the dialog"
    );
    let addr: SocketAddr = "127.0.0.1:34567".parse().unwrap();
    assert!(
        messages(&mut f)
            .iter()
            .any(|m| m.contains(&format!("http://{addr}"))),
        "{:?}",
        messages(&mut f)
    );
    let toast = toasts(&mut f).pop().expect("a toast");
    let action: &'static str = Box::leak(format!("toast-{}-action-0", toast.id.as_u64()).into());
    assert!(shown(&mut f, action), "the toast offers Open in browser");
}

#[gpui::test]
fn auto_sends_no_local_port(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    let d = open_dialog(&mut f, &table);
    f.settle();
    f.vcx.update(|window, cx| {
        d.update(cx, |d, cx| d.fill("8080", "", "127.0.0.1", window, cx));
    });
    click(&mut f, "pf-submit");
    let starts = f.forward.as_ref().unwrap().starts();
    assert!(
        matches!(
            &starts[..],
            [Command::PortForwardStart {
                local_port: None,
                remote_port: ForwardPort::Number(8080),
                ..
            }]
        ),
        "{starts:?}"
    );
}

#[gpui::test]
fn invalid_fields_say_why_and_keep_forward_off(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    let d = open_dialog(&mut f, &table);
    f.settle();
    f.vcx.update(|window, cx| {
        d.update(cx, |d, cx| d.fill("8080", "70000", "not-an-ip", window, cx));
    });
    f.settle();
    assert!(shown(&mut f, "pf-local-problem"));
    assert!(shown(&mut f, "pf-bind-problem"));
    assert!(!f.vcx.update(|_, cx| d.read(cx).can_submit(cx)));
    f.vcx.update(|_, cx| d.update(cx, |d, cx| d.submit(cx)));
    f.settle();
    assert!(f.forward.as_ref().unwrap().calls.lock().is_empty());
    assert!(dialog(&mut f).is_some());
}

#[gpui::test]
fn a_non_loopback_bind_warns_and_is_remembered_for_the_next_dialog(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    let d = open_dialog(&mut f, &table);
    f.settle();
    f.vcx.update(|window, cx| {
        d.update(cx, |d, cx| d.fill("8080", "", "0.0.0.0", window, cx));
    });
    f.settle();
    assert!(shown(&mut f, "pf-bind-warning"));
    click(&mut f, "pf-submit");
    assert!(dialog(&mut f).is_none());

    let d = open_dialog(&mut f, &table);
    let input = f.vcx.update(|_, cx| d.read(cx).input(cx));
    assert_eq!(
        input.bind, "0.0.0.0",
        "the last bind address is the default"
    );
    assert!(input.local_auto);
}

#[gpui::test]
fn a_failed_start_stays_open_with_the_message_the_suggestion_and_the_fix(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    let busy = oxikube_ports::PortForwardError::BindFailed {
        addr: "127.0.0.1:18080".parse().unwrap(),
        kind: std::io::ErrorKind::AddrInUse,
    };
    *f.forward.as_ref().unwrap().reply.lock() = Reply::Fail(StartError::Forward(busy));
    let d = open_dialog(&mut f, &table);
    f.settle();
    f.vcx.update(|window, cx| {
        d.update(cx, |d, cx| d.fill("8080", "18080", "127.0.0.1", window, cx));
    });
    click(&mut f, "pf-submit");

    assert!(
        dialog(&mut f).is_some(),
        "the dialog stays for a second try"
    );
    assert!(shown(&mut f, "pf-problem-message"));
    assert!(shown(&mut f, "pf-problem-suggestion"));
    let problem = f
        .vcx
        .update(|_, cx| d.read(cx).problem().cloned())
        .expect("a problem");
    assert!(
        problem
            .suggestion
            .unwrap()
            .contains("Use Auto or choose another local port")
    );
    assert_eq!(problem.fix, Some(Fix::UseAuto));
    assert_eq!(f.vcx.update(|_, cx| d.read(cx).stage()), Stage::Editing);

    // One click: Auto. The next submit sends no local port and, now that the port is free, works.
    *f.forward.as_ref().unwrap().reply.lock() = Reply::Up;
    click(&mut f, "pf-problem-fix");
    assert!(f.vcx.update(|_, cx| d.read(cx).input(cx).local_auto));
    click(&mut f, "pf-submit");
    let starts = f.forward.as_ref().unwrap().starts();
    assert!(matches!(
        starts.last(),
        Some(Command::PortForwardStart {
            local_port: None,
            ..
        })
    ));
    assert!(dialog(&mut f).is_none());
}

#[gpui::test]
fn a_read_only_cluster_blocks_the_dialog_with_the_reason_unless_allowed(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    let rig = f.forward.clone().unwrap();
    rig.read_only
        .store(true, std::sync::atomic::Ordering::SeqCst);
    f.keys(&table, "shift-f");
    assert!(dialog(&mut f).is_none());
    assert!(
        messages(&mut f)
            .iter()
            .any(|m| m.contains("allowed_in_readonly")),
        "{:?}",
        messages(&mut f)
    );

    rig.manager.set_config(ForwardConfig {
        allowed_in_readonly: true,
        ..Default::default()
    });
    f.keys(&table, "shift-f");
    assert!(dialog(&mut f).is_some(), "the setting lets it through");
}

#[gpui::test]
fn escape_cancels_and_enter_submits(cx: &mut TestAppContext) {
    let (mut f, table) = open(cx);
    let _d = open_dialog(&mut f, &table);
    f.settle();
    f.vcx.simulate_keystrokes("escape");
    f.settle();
    assert!(dialog(&mut f).is_none(), "Escape cancels");
    assert!(f.forward.as_ref().unwrap().calls.lock().is_empty());

    let _d = open_dialog(&mut f, &table);
    f.settle();
    f.vcx.simulate_keystrokes("enter");
    f.settle();
    assert_eq!(
        f.forward.as_ref().unwrap().starts().len(),
        1,
        "Enter submits"
    );
    assert!(dialog(&mut f).is_none());
}
