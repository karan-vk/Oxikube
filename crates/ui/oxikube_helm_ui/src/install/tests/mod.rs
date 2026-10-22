//! Tests of the install tab: the pure parts, and the tab in a workspace window over the real
//! `InstallFlow` and `ChartsService`, a real `MutationGuard` and command bus, a connected fake
//! cluster and the testkit's `FakeHelmPort`.

mod page;
mod parts;

use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{AppContext as _, Entity, Modifiers, Point, TestAppContext, VisualTestContext};
use oxikube_app::command_bus::CommandRegistry;
use oxikube_app::session::SessionOptions;
use oxikube_app::{
    ChartsService, ClusterSessionManager, CommandBus, HelmReposService, InstallFlow, MutationGuard,
    ReleaseProgress,
};
use oxikube_domain::helm::{ChartHit, ChartRef, RepoEntry};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, ListPage, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeHelmPort, FakeStatePort,
    resource,
};
use oxikube_workspace::OpenOptions;
use oxikube_workspace::test_support::open_workspace;

use super::{FlowBackend, HelmInstallView, InstallDeps};

/// The chart's default values: helm-docs comments, a password key.
pub(super) const DEFAULTS: &str = "# -- Number of replicas\nreplicaCount: 1\nimage:\n  # -- (string) Image repository\n  repository: bitnami/nginx\nservice:\n  # -- How the Service is exposed\n  type: ClusterIP\nauth:\n  password: \"\"\n";

/// What `helm template` renders: a Service and a Secret.
pub(super) const RENDERED: &str = "---\n# Source: nginx/templates/svc.yaml\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n---\n# Source: nginx/templates/secret.yaml\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-auth\ndata:\n  password: c3VwZXJzZWNyZXQ=\n";

fn context() -> ClusterContext {
    let name = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &name);
    ClusterContext::new(cluster, name, SourceId("kubeconfig".into()))
}

pub(super) fn cluster() -> ClusterId {
    context().cluster
}

pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub view: Entity<HelmInstallView>,
    pub fake: Arc<FakeHelmPort>,
    pub state: Arc<FakeStatePort>,
}

/// How a harness starts.
pub(super) struct Options {
    pub read_only: bool,
    pub with_helm: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            read_only: false,
            with_helm: true,
        }
    }
}

impl Harness {
    pub fn open(cx: &mut TestAppContext) -> Self {
        Self::open_with(cx, Options::default())
    }

    pub fn open_with(cx: &mut TestAppContext, options: Options) -> Self {
        let fake = Arc::new(if options.with_helm {
            FakeHelmPort::new()
        } else {
            FakeHelmPort::native_only()
        });
        // `ready` (repos), the defaults (show values), the versions (search).
        fake.script().repos.push_ok(vec![RepoEntry {
            name: "bitnami".into(),
            url: "https://charts.bitnami.com/bitnami".into(),
        }]);
        fake.script().show.push_ok(DEFAULTS.to_owned());
        fake.script().search.push_ok(
            ["15.4.0", "15.3.1"]
                .into_iter()
                .map(|v| ChartHit {
                    chart: ChartRef::new("nginx", v).in_repo("bitnami"),
                    app_version: Some("1.25.3".into()),
                    description: "NGINX".into(),
                })
                .collect(),
        );

        let connector = Arc::new(FakeClusterConnectorPort::new());
        connector
            .ports_for(&cluster())
            .resources
            .script()
            .list
            .push_ok(ListPage {
                items: ["default", "demo", "kube-system"]
                    .into_iter()
                    .map(|n| resource("v1", "Namespace").name(n).cluster_scoped().build())
                    .collect(),
                continue_token: None,
                resource_version: None,
                remaining_item_count: None,
            });
        let source = Arc::new(FakeClusterSourcePort::new().with_contexts([context()]));
        let clock = Arc::new(FakeClockPort::default());
        let sessions = ClusterSessionManager::new(connector, source, clock.clone());
        sessions.open(
            &context(),
            SessionOptions {
                read_only: options.read_only,
                ..SessionOptions::default()
            },
        );
        sessions
            .connect(&cluster())
            .now_or_never()
            .expect("the fake connects at once")
            .expect("connected");
        let state = Arc::new(FakeStatePort::new());
        let progress = ReleaseProgress::new();
        let mut registry = CommandRegistry::new();
        let (port, follow) = (fake.clone(), progress.clone());
        registry
            .install("oxikube_app::helm_install", |r| {
                oxikube_app::helm_install::register_commands(r, port, follow)
            })
            .expect("registers");
        let guard = MutationGuard::new(sessions.clone(), state.clone(), clock.clone());
        let bus = CommandBus::new(registry, guard);
        let flow = InstallFlow::new(bus, sessions, progress, "me");
        let repos = HelmReposService::new(fake.clone(), fake.clone(), clock);
        let charts = ChartsService::new(fake.clone(), repos.catalog_changes());

        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
        let deps = InstallDeps {
            backend: Rc::new(FlowBackend::new(flow, charts)),
            cluster: cluster(),
            cluster_label: "kind-oxikube".into(),
        };
        let chart = ChartRef::new("nginx", "15.4.0").in_repo("bitnami");
        let mut view = None;
        vcx.update(|window, cx| {
            let entity = cx.new(|cx| HelmInstallView::new(deps, chart, window, cx));
            view = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        let mut harness = Self {
            vcx,
            view: view.expect("built"),
            fake,
            state,
        };
        harness.settle();
        harness
    }

    /// Runs what is ready and lets the redraws land.
    pub fn settle(&mut self) {
        for _ in 0..6 {
            self.vcx.run_until_parked();
        }
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&HelmInstallView, &gpui::App) -> R) -> R {
        let view = self.view.clone();
        self.vcx.read(|cx| f(view.read(cx), cx))
    }

    pub fn update<R>(
        &mut self,
        f: impl FnOnce(
            &mut HelmInstallView,
            &mut gpui::Window,
            &mut gpui::Context<HelmInstallView>,
        ) -> R,
    ) -> R {
        let view = self.view.clone();
        let out = self
            .vcx
            .update(|window, cx| view.update(cx, |view, cx| f(view, window, cx)));
        self.settle();
        out
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector).is_some()
    }

    pub fn centre(&mut self, selector: &str) -> Point<gpui::Pixels> {
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center()
    }

    pub fn click(&mut self, selector: &str) {
        let at = self.centre(selector);
        self.vcx.simulate_click(at, Modifiers::none());
        self.settle();
    }

    /// Replaces the text of the release name field.
    pub fn set_release(&mut self, text: &str) {
        let input = self.read(|v, _| v.release.clone());
        let text = text.to_owned();
        self.vcx
            .update(|window, cx| input.update(cx, |i, cx| i.set_value(text, window, cx)));
        self.settle();
    }

    /// Replaces the values in the editor.
    pub fn set_values(&mut self, text: &str) {
        self.update(|view, window, cx| view.set_values(text, window, cx));
    }

    /// Scripts a passing preview (the template, then the server's dry run).
    pub fn script_preview(&self) {
        self.fake.script().template.push_ok(RENDERED.to_owned());
        self.fake.script().install.push_ok(Vec::new());
    }

    pub fn audit(&self) -> Vec<oxikube_domain::audit::AuditRecord> {
        self.state.audit_log()
    }
}
