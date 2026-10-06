//! [`ResourceViews`]: the sidebar and `resource::OpenList` open a kind's table in its cluster
//! tab, and the table commands on a real `CommandBus`.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::{CommandBus, CommandRegistry, DispatchContext};
use oxikube_app::{ClusterSessionManager, MutationGuard};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
};
use oxikube_workspace::sidebar::{SidebarEvent, SidebarPanel, SidebarTarget};

use super::fixture::{Fixture, cluster};
use super::{p, pods_kind};
use crate::navigate::OpenKind;
use crate::table::ResourceTable;
use crate::views::find_kind;
use crate::{RESOURCE_COMMANDS, ResourceCommandSink, ViewRequest, register_commands};

#[gpui::test]
fn a_sidebar_entry_opens_the_kind_in_the_cluster_tab(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "web-0", "1"), p("x", "web-1", "1")]);
    let tab = f
        .vcx
        .update(|_, cx| f.tabs.read(cx).tab(&cluster()).cloned())
        .expect("a tab");
    let inner = f.vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let panel = f
        .vcx
        .update(|_, cx| inner.read(cx).panel::<SidebarPanel>())
        .expect("the tab has its sidebar");

    let navigate = |f: &mut Fixture| {
        f.vcx.update(|_, cx| {
            panel.update(cx, |_, cx| {
                cx.emit(SidebarEvent::Navigate(SidebarTarget::kind("", "pods")));
            })
        });
        f.settle();
    };
    navigate(&mut f);
    assert!(f.dispatcher.sent().contains(&Command::ResourceOpenList {
        cluster: cluster(),
        gvk: Gvk::new("", "v1", "Pod"),
    }));
    let tables = f
        .vcx
        .update(|_, cx| inner.read(cx).items_of_type::<ResourceTable>());
    assert_eq!(
        tables.len(),
        1,
        "the table opened in the cluster's own workspace"
    );
    assert_eq!(f.names(&tables[0]), ["web-0", "web-1"]);
    let title = f
        .vcx
        .update(|_, cx| tables[0].read(cx).gvk().kind.to_string());
    assert_eq!(title, "Pod");

    // Again: the open table is shown, not a second one.
    navigate(&mut f);
    let tables = f
        .vcx
        .update(|_, cx| inner.read(cx).items_of_type::<ResourceTable>());
    assert_eq!(tables.len(), 1);
}

#[gpui::test]
fn copy_name_writes_the_clipboard(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([]);
    let views = f.views.clone();
    let target = ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "x", "web-0");
    f.vcx.update(|window, cx| {
        views.update(cx, |views, cx| {
            views.apply_in(ViewRequest::CopyName(target), window, cx)
        })
    });
    let text = f
        .vcx
        .update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(text.as_deref(), Some("web-0"));
}

#[test]
fn kinds_are_found_by_group_and_plural_preferring_the_preferred_version() {
    let mut old = pods_kind();
    old.gvk = Gvk::new("", "v0", "Pod");
    old.preferred = false;
    let kinds = [old, pods_kind()];
    assert_eq!(
        find_kind(&kinds, "", "pods").unwrap().gvk.version.as_ref(),
        "v1"
    );
    assert!(find_kind(&kinds, "apps", "pods").is_none());
}

#[test]
fn the_commands_reach_the_views_through_the_bus() {
    let entry = ClusterContext::new(
        cluster(),
        oxikube_domain::ids::ContextName::new("kind"),
        SourceId("kubeconfig".into()),
    );
    let clock = Arc::new(FakeClockPort::default());
    let connector = Arc::new(FakeClusterConnectorPort::new());
    connector
        .ports_for(&cluster())
        .discovery
        .set_kinds([pods_kind()]);
    let source = Arc::new(FakeClusterSourcePort::new().with_contexts([entry]));
    let sessions = ClusterSessionManager::new(connector, source, clock.clone());
    block_on(sessions.connect(&cluster())).unwrap();
    let (sink, mut requests) = ResourceCommandSink::channel();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_resources_ui", |r| register_commands(r, sink))
        .unwrap();
    let sessions_handle = sessions.clone();
    let guard = MutationGuard::new(sessions, Arc::new(FakeStatePort::new()), clock);
    let bus = CommandBus::new(registry, guard);
    for id in RESOURCE_COMMANDS {
        assert!(bus.is_registered(id), "{id}");
        assert!(bus.tool(id).is_some(), "{id} has an MCP tool stub");
    }
    let ctx = || DispatchContext::new(Initiator::Ui, "me");
    let pods = Gvk::new("", "v1", "Pod");

    assert!(
        !bus.is_registered(oxikube_domain::command::CommandId::RESOURCE_OPEN_LIST),
        "resource::OpenList is navigate's (E07-S11), not a table command"
    );

    let target = ResourceRef::namespaced(cluster(), pods.clone(), "x", "web-0");
    block_on(bus.dispatch(
        Command::ResourceOpen {
            target: target.clone(),
        },
        ctx(),
    ))
    .unwrap();
    assert_eq!(
        requests.try_recv().ok(),
        Some(ViewRequest::Open(target.clone()))
    );
    block_on(bus.dispatch(
        Command::ResourceCopyName {
            target: target.clone(),
        },
        ctx(),
    ))
    .unwrap();
    assert_eq!(
        requests.try_recv().ok(),
        Some(ViewRequest::CopyName(target.clone()))
    );
    block_on(bus.dispatch(
        Command::ResourcePinDetail {
            target: target.clone(),
        },
        ctx(),
    ))
    .unwrap();
    assert_eq!(
        requests.try_recv().ok(),
        Some(ViewRequest::PinDetail(target.clone()))
    );
    block_on(bus.dispatch(
        Command::ResourceCopyLabel {
            target: target.clone(),
            key: "app".into(),
            annotation: false,
        },
        ctx(),
    ))
    .unwrap();
    assert_eq!(
        requests.try_recv().ok(),
        Some(ViewRequest::CopyLabel {
            target: target.clone(),
            key: "app".into(),
            annotation: false,
        })
    );
    // The YAML and Describe tabs' commands (E07-S06).
    for (command, request) in [
        (
            Command::ResourceCopyYaml {
                target: target.clone(),
            },
            ViewRequest::CopyYaml(target.clone()),
        ),
        (
            Command::ResourceSaveYaml {
                target: target.clone(),
            },
            ViewRequest::SaveYaml(target.clone()),
        ),
        (
            Command::ResourceToggleManagedFields {
                target: target.clone(),
            },
            ViewRequest::ToggleManagedFields(target.clone()),
        ),
        (
            Command::ResourceRefreshDescribe {
                target: target.clone(),
            },
            ViewRequest::RefreshDescribe(target),
        ),
    ] {
        block_on(bus.dispatch(command, ctx())).unwrap();
        assert_eq!(requests.try_recv().ok(), Some(request));
    }
    block_on(bus.dispatch(
        Command::ResourceSelectAll {
            cluster: cluster(),
            gvk: pods.clone(),
        },
        ctx(),
    ))
    .unwrap();
    assert_eq!(
        requests.try_recv().ok(),
        Some(ViewRequest::SelectAll {
            cluster: cluster(),
            gvk: pods.clone(),
        })
    );
    // `table::FocusFilter` moves focus only: no guard tier, so it also runs in read-only mode.
    sessions_handle
        .set_read_only(&cluster(), true)
        .expect("open session");
    block_on(bus.dispatch(
        Command::TableFocusFilter {
            cluster: cluster(),
            gvk: pods.clone(),
        },
        ctx(),
    ))
    .unwrap();
    assert_eq!(
        requests.try_recv().ok(),
        Some(ViewRequest::FocusFilter {
            cluster: cluster(),
            gvk: pods,
        })
    );
}

/// The tables open in the cluster tab's workspace.
fn tab_tables(f: &mut Fixture) -> Vec<gpui::Entity<ResourceTable>> {
    let tab = f
        .vcx
        .update(|_, cx| f.tabs.read(cx).tab(&cluster()).cloned())
        .expect("a tab");
    f.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<ResourceTable>()
    })
}

fn open_list(f: &mut Fixture, gvk: Gvk) {
    let dispatcher = f.deps.dispatcher.clone();
    f.vcx.update(|_, cx| {
        dispatcher.dispatch(
            Command::ResourceOpenList {
                cluster: cluster(),
                gvk,
            },
            cx,
        )
    });
    f.settle();
}

#[gpui::test]
fn open_list_resolves_the_kind_through_discovery_and_opens_its_table(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([p("x", "web-0", "1")]);
    // No sidebar click first: nothing is cached, so the kind view asks discovery.
    open_list(&mut f, Gvk::new("", "v1", "Pod"));
    let tables = tab_tables(&mut f);
    assert_eq!(tables.len(), 1, "the kind view opened the table");
    assert_eq!(f.names(&tables[0]), ["web-0"]);
    // Again (a tile, the palette, an agent): the open table is shown, not a second one.
    open_list(&mut f, Gvk::new("", "v1", "Pod"));
    assert_eq!(tab_tables(&mut f).len(), 1);
}

#[gpui::test]
fn open_list_for_a_kind_the_cluster_does_not_serve_says_so(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([]);
    open_list(&mut f, Gvk::new("example.com", "v1", "Widget"));
    assert!(tab_tables(&mut f).is_empty());
    let tab = f
        .vcx
        .update(|_, cx| f.tabs.read(cx).tab(&cluster()).cloned())
        .expect("a tab");
    let toasts: Vec<String> = f.vcx.update(|_, cx| {
        let ws = tab.read(cx).workspace().clone();
        let layer = ws.read(cx).toast_layer().clone();
        layer
            .read(cx)
            .visible()
            .into_iter()
            .map(|t| t.message.to_string())
            .collect()
    });
    assert!(
        toasts
            .iter()
            .any(|t| t == "The cluster does not serve Widget."),
        "{toasts:?}"
    );
}

#[gpui::test]
fn the_kind_view_takes_only_a_connected_cluster_tab_of_its_window(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    f.connect_with([]);
    let tab = f
        .vcx
        .update(|_, cx| f.tabs.read(cx).tab(&cluster()).cloned())
        .expect("a tab");
    let tab_workspace = f.vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let request = OpenKind {
        cluster: cluster(),
        gvk: Gvk::new("", "v1", "Pod"),
    };
    let ask = |f: &mut Fixture, workspace: &gpui::Entity<oxikube_workspace::Workspace>| {
        let views = f.views.clone();
        f.vcx.update(|window, cx| {
            views.update(cx, |views, cx| {
                views.open_kind(&request, workspace, window, cx)
            })
        })
    };
    // Not the cluster's tab (another window's views ask the same registry): left to others.
    let window_workspace = f.window_workspace.clone();
    assert!(!ask(&mut f, &window_workspace));
    assert!(
        ask(&mut f, &tab_workspace),
        "its own tab of a connected cluster"
    );
    f.settle();
    // Disconnected (the tab closes): the window shows its "no list view" notice instead.
    f.sessions.disconnect(&cluster()).expect("disconnect");
    f.settle();
    assert!(!ask(&mut f, &tab_workspace));
}
