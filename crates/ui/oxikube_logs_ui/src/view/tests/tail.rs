//! "Tail in terminal" (E08-S08): `kubectl logs -f` for what the view shows, in a terminal tab of
//! the cluster. Over a fake kubectl lookup and the window's real `TerminalViews` on a fake
//! launcher: no process, no cluster.

use std::path::PathBuf;
use std::rc::Rc;

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{App, Entity, Task, TestAppContext, Window};
use oxikube_domain::OxiResult;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_ports::TerminalSize;
use oxikube_terminal::view::{
    BackendDescriptor, Launch, TerminalHost, TerminalLauncher, TerminalRequest, TerminalServices,
    TerminalView, TerminalViews, TerminalViewsDeps,
};
use oxikube_testkit::Timeline;
use oxikube_testkit::fakes::FakeTerminalBackend;
use oxikube_workspace::{Item as _, Workspace};

use super::fixture::{Fx, KUBECTL, cluster, deployment_ref, lines, pod_line, pod_ref};
use crate::LogView;
use crate::view::OpenLogs;

fn open(fx: &mut Fx) -> Entity<LogView> {
    fx.open(Timeline::immediate(lines(0, 5)).keep_open())
}

/// The descriptor the next request asks a terminal for.
fn requested(fx: &mut Fx) -> BackendDescriptor {
    fx.vcx.run_until_parked();
    match fx.terminal_requests.try_recv() {
        Ok(TerminalRequest::Open { descriptor }) => descriptor,
        other => panic!("a terminal was requested, got {other:?}"),
    }
}

/// Whether the toolbar's "..." menu offers "Tail in terminal" now.
fn offered(fx: &mut Fx, view: &Entity<LogView>) -> bool {
    fx.overflow_ids(view).contains(&"log-tail-in-terminal")
}

fn nothing_requested(fx: &mut Fx) -> bool {
    fx.vcx.run_until_parked();
    fx.terminal_requests.try_recv().is_err()
}

/// `program`, `args` and the tab title of a local descriptor.
fn command_of(descriptor: &BackendDescriptor) -> (String, Vec<String>, String) {
    let BackendDescriptor::Local {
        shell, args, title, ..
    } = descriptor
    else {
        panic!("a local process: {descriptor:?}");
    };
    (
        shell.clone().expect("kubectl is the program"),
        args.clone(),
        title.clone().expect("a title"),
    )
}

#[gpui::test]
fn the_action_is_in_the_toolbar_and_sends_its_command(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    fx.draw();
    assert!(offered(&mut fx, &view), "kubectl is installed");
    fx.overflow("log-tail-in-terminal");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsTailInTerminal { target: pod_ref() }),
        "the button is the command"
    );
    requested(&mut fx);
}

#[gpui::test]
fn it_is_hidden_not_disabled_without_kubectl(cx: &mut TestAppContext) {
    let mut fx = Fx::without_kubectl(cx);
    let view = open(&mut fx);
    fx.draw();
    assert!(!offered(&mut fx, &view), "no entry at all");
    assert!(!fx.read(&view, LogView::can_tail_in_terminal));
    // Every other control is still there.
    let ids = fx.overflow_ids(&view);
    assert!(ids.contains(&"log-copy") && ids.contains(&"log-save"));

    // The key and the palette's command say why instead of doing nothing.
    fx.keys("shift-t");
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::LogsTailInTerminal { target: pod_ref() })
    );
    assert!(nothing_requested(&mut fx), "no terminal opens");
    assert!(
        fx.toasts()
            .iter()
            .any(|t| t == "kubectl was not found on this machine."),
        "{:?}",
        fx.toasts()
    );
}

#[gpui::test]
fn installing_kubectl_shows_the_action_without_restarting(cx: &mut TestAppContext) {
    let mut fx = Fx::without_kubectl(cx);
    let view = open(&mut fx);
    fx.draw();
    assert!(!offered(&mut fx, &view));

    // The user installs kubectl while the app runs. The answer is cached: nothing looks yet.
    *fx.installed.lock() = Some(PathBuf::from(KUBECTL));
    fx.draw();
    assert!(!offered(&mut fx, &view));
    assert!(!fx.read(&view, LogView::can_tail_in_terminal));

    // The app looks again now and then, off the UI thread (`follow_kubectl`).
    let _follow = fx
        .vcx
        .update(|_, cx| crate::follow_kubectl(&fx.kubectl, cx));
    fx.vcx.run_until_parked();
    fx.vcx.executor().advance_clock(crate::KUBECTL_POLL);
    fx.vcx.run_until_parked();
    fx.draw();
    assert!(fx.read(&view, LogView::can_tail_in_terminal));
    assert!(offered(&mut fx, &view), "the menu offers it now");

    // And it is forgotten again when kubectl goes away.
    *fx.installed.lock() = None;
    fx.vcx.executor().advance_clock(crate::KUBECTL_POLL);
    fx.vcx.run_until_parked();
    fx.draw();
    assert!(!offered(&mut fx, &view));
}

#[gpui::test]
fn a_pod_is_tailed_with_kubectl_in_its_cluster_and_namespace(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    let container = fx
        .read(&view, |v| v.options().container.clone())
        .expect("the pod's default container is read");
    fx.keys("shift-t");

    let descriptor = requested(&mut fx);
    let (program, args, title) = command_of(&descriptor);
    assert_eq!(
        program, KUBECTL,
        "the kubectl that was found, by absolute path"
    );
    assert_eq!(
        args,
        [
            "logs".to_owned(),
            "-f".into(),
            "--context=kind".into(),
            "--namespace=shop".into(),
            format!("--container={container}"),
            "--tail=1000".into(),
            "web-0".into(),
        ]
    );
    assert_eq!(title, format!("logs web-0/{container}"));
    // The cluster and namespace are what the terminal's environment is built from.
    let BackendDescriptor::Local {
        cluster: tab_cluster,
        namespace,
        ..
    } = &descriptor
    else {
        unreachable!()
    };
    assert_eq!(tab_cluster.as_ref(), Some(&cluster()));
    assert_eq!(namespace.as_deref(), Some("shop"));
    // Nothing secret is in what the tab keeps.
    let saved = descriptor.to_state().to_string();
    for secret in ["KUBECONFIG", "token", "password", "client-key"] {
        assert!(!saved.contains(secret), "{secret} in {saved}");
    }
}

#[gpui::test]
fn the_views_options_become_kubectls(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    let view = open(&mut fx);
    fx.keys("t"); // timestamps
    fx.keys("3"); // the last 5 minutes
    fx.keys("shift-t");
    let (_, args, _) = command_of(&requested(&mut fx));
    for flag in ["-f", "--timestamps", "--since=300s"] {
        assert!(args.iter().any(|a| a == flag), "{flag} in {args:?}");
    }
    assert!(!args.iter().any(|a| a.starts_with("--tail")), "{args:?}");
    assert_eq!(args.last().map(String::as_str), Some("web-0"));

    // The previous instance is read once, not followed.
    fx.keys("p");
    fx.keys("shift-t");
    let (_, args, _) = command_of(&requested(&mut fx));
    assert!(args.iter().any(|a| a == "--previous"), "{args:?}");
    assert!(!args.iter().any(|a| a == "-f"), "{args:?}");
    let _ = view;
}

#[gpui::test]
fn a_workloads_pods_are_tailed_by_selector_with_their_names(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    fx.seed_web(vec![
        (
            "web-7d9-aaaaa",
            Timeline::immediate([pod_line("web-7d9-aaaaa", 0)]).keep_open(),
        ),
        (
            "web-7d9-bbbbb",
            Timeline::immediate([pod_line("web-7d9-bbbbb", 1)]).keep_open(),
        ),
    ]);
    let view = fx.open_web();
    fx.overflow("log-tail-in-terminal");

    let (program, args, title) = command_of(&requested(&mut fx));
    assert_eq!(program, KUBECTL);
    assert_eq!(
        args,
        [
            "logs",
            "-f",
            "--context=kind",
            "--namespace=shop",
            "--all-containers=true",
            "--tail=1000",
            "--selector=app=web",
            "--prefix",
            "--max-log-requests=20"
        ]
    );
    assert_eq!(title, "logs deployment/web");
    let _ = view;
}

#[gpui::test]
fn a_workload_whose_selector_is_not_read_yet_says_so(cx: &mut TestAppContext) {
    let mut fx = Fx::merged(cx);
    // No deployment in the cluster: its selector never resolves.
    let views = fx.views.clone();
    let view = fx.vcx.update(|window, cx| {
        views.update(cx, |views, cx| {
            views.open(&deployment_ref(), &OpenLogs::default(), window, cx)
        })
    });
    fx.settle();
    assert!(view.is_some(), "the tab is open");
    fx.keys("shift-t");
    assert!(nothing_requested(&mut fx), "nothing to run yet");
    assert!(
        fx.toasts()
            .iter()
            .any(|t| t == "The pods of this view are still being looked up; try again in a moment."),
        "{:?}",
        fx.toasts()
    );
}

// --- the window's real terminal views ---------------------------------------------------------

/// Starts a fake process for every launch and remembers what it was asked.
#[derive(Default)]
struct Launcher(std::cell::RefCell<Vec<BackendDescriptor>>);

impl TerminalLauncher for Launcher {
    fn launch(&self, descriptor: &BackendDescriptor, _: TerminalSize, _: &mut App) -> Launch {
        self.0.borrow_mut().push(descriptor.clone());
        let started: OxiResult<Box<dyn oxikube_ports::TerminalBackend>> =
            Ok(Box::new(FakeTerminalBackend::silent()));
        Task::ready(started)
    }
}

struct TabHost(Entity<Workspace>);

impl TerminalHost for TabHost {
    fn active_cluster(&self, _: &App) -> Option<ClusterId> {
        Some(cluster())
    }
    fn workspace(&self, id: &ClusterId, _: &App) -> Option<Entity<Workspace>> {
        (id == &cluster()).then(|| self.0.clone())
    }
    fn show(&self, _: &ClusterId, _: &mut Window, _: &mut App) {}
}

/// Hands the requests the views asked for to the window's `TerminalViews`.
fn start_terminals(
    fx: &mut Fx,
    requests: UnboundedReceiver<TerminalRequest>,
) -> (Rc<Launcher>, Entity<TerminalViews>) {
    let launcher = Rc::new(Launcher::default());
    let services = TerminalServices::new(launcher.clone());
    let deps = TerminalViewsDeps {
        host: Rc::new(TabHost(fx.workspace.clone())),
        window_workspace: fx.workspace.downgrade(),
        services,
    };
    let views = fx
        .vcx
        .update(|window, cx| TerminalViews::start(deps, requests, window, cx));
    (launcher, views)
}

#[gpui::test]
fn clicking_the_action_opens_a_terminal_tab_running_kubectl(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx);
    // The window's terminal views apply the requests the log view makes.
    let (_unused, requests) = oxikube_terminal::view::TerminalViewSink::channel();
    let (launcher, terminals) = start_terminals(&mut fx, requests);
    open(&mut fx);
    fx.draw();
    fx.overflow("log-tail-in-terminal");
    let request = fx
        .terminal_requests
        .try_recv()
        .expect("the view asked for a terminal");
    fx.vcx.update(|window, cx| {
        terminals.update(cx, |views, cx| views.apply(request, window, cx));
    });
    fx.vcx.run_until_parked();
    fx.vcx
        .executor()
        .advance_clock(oxikube_runtime::FRAME_INTERVAL);
    fx.vcx.run_until_parked();

    let launches = launcher.0.borrow().clone();
    let [descriptor] = &launches[..] else {
        panic!("one process started: {launches:?}");
    };
    let (program, args, _) = command_of(descriptor);
    assert_eq!(program, KUBECTL);
    assert_eq!(args[..2], ["logs", "-f"]);
    assert_eq!(args.last().map(String::as_str), Some("web-0"));
    // The launcher builds the environment from the cluster and namespace (KUBECONFIG, context),
    // so the descriptor names them and carries no value.
    assert_eq!(descriptor.cluster(), Some(&cluster()));

    let ws = fx.workspace.clone();
    let tabs = fx
        .vcx
        .update(|_, cx| ws.read(cx).items_of_type::<TerminalView>());
    assert_eq!(tabs.len(), 1, "a terminal tab of the cluster");
    let title = fx
        .vcx
        .update(|_, cx| tabs[0].read(cx).tab_content(cx).title.to_string());
    assert!(title.starts_with("logs web-0"), "{title}");
}
