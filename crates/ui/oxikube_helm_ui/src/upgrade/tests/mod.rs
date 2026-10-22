//! Tests of the upgrade tab: the pure parts, and the tab in a workspace window over the real
//! `UpgradeFlow` and `ChartsService`, a real `MutationGuard` and command bus, a connected fake
//! cluster and the testkit's `FakeHelmPort`.

mod page;
mod parts;

use std::rc::Rc;
use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{AppContext as _, Entity, Modifiers, Point, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use oxikube_app::command_bus::CommandRegistry;
use oxikube_app::session::SessionOptions;
use oxikube_app::{
    ChartsService, ClusterSessionManager, CommandBus, HelmReposService, MutationGuard,
    ReleaseProgress, UpgradeFlow, UpgradeRequests,
};
use oxikube_domain::helm::{
    ChartHit, ChartRef, Release, ReleaseRef, ReleaseStatus, RepoEntry, Revision, Values,
};
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeHelmPort, FakeStatePort,
};
use oxikube_workspace::OpenOptions;
use oxikube_workspace::test_support::open_workspace;

use super::{FlowBackend, HelmUpgradeView, UpgradeDeps};

/// The release's user-supplied values: replicas, an image and a password.
pub(super) const CURRENT_VALUES: &str =
    "replicaCount: 2\nimage:\n  tag: old\nauth:\n  password: hunter2-value\n";

/// The chart's defaults at the version the release runs.
pub(super) const DEFAULTS_OLD: &str =
    "# -- Number of replicas\nreplicaCount: 1\nimage:\n  tag: latest\nservice:\n  port: 80\n";

/// The chart's defaults at the newest version: `service.port` renamed, `ingress` added.
pub(super) const DEFAULTS_NEW: &str = "# -- Number of replicas\nreplicaCount: 1\nimage:\n  tag: latest\nservice:\n  httpPort: 80\ningress:\n  enabled: false\n";

/// What the release runs now.
pub(super) const CURRENT_MANIFEST: &str = "---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 2\n---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-auth\ndata:\n  password: b2xkcGFzcw==\n";

/// What the new chart renders: the Deployment changed, an Ingress added, the Secret gone.
pub(super) const RENDERED: &str = "---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 3\n---\napiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: web\n";

fn context() -> ClusterContext {
    let name = ContextName::new("kind-oxikube");
    let cluster = ClusterId::new("/home/me/.kube/config", &name);
    ClusterContext::new(cluster, name, SourceId("kubeconfig".into()))
}

pub(super) fn cluster() -> ClusterId {
    context().cluster
}

pub(super) fn web() -> ReleaseRef {
    ReleaseRef::new(cluster(), "demo", "web")
}

pub(super) fn revision(number: u32, version: &str, status: ReleaseStatus) -> Revision {
    Revision {
        revision: number,
        status,
        chart: ChartRef::new("nginx", version),
        app_version: Some("1.25.0".into()),
        updated: Timestamp::UNIX_EPOCH,
        description: String::new(),
    }
}

pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub view: Entity<HelmUpgradeView>,
    pub fake: Arc<FakeHelmPort>,
    pub state: Arc<FakeStatePort>,
}

/// How a harness starts.
pub(super) struct Options {
    pub read_only: bool,
    pub with_helm: bool,
    /// Open on a release (else the picker).
    pub release: Option<ReleaseRef>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            read_only: false,
            with_helm: true,
            release: Some(web()),
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
        // `ready` (repos), the release (history, user values), the sources (search), the defaults
        // at the running version and at the newest (show).
        fake.script().repos.push_ok(vec![RepoEntry {
            name: "bitnami".into(),
            url: "https://charts.bitnami.com/bitnami".into(),
        }]);
        fake.script()
            .history
            .push_ok(vec![revision(3, "15.0.0", ReleaseStatus::Deployed)]);
        fake.script()
            .values
            .push_ok(Values::from_yaml(CURRENT_VALUES).unwrap());
        fake.script().search.push_ok(
            ["15.4.0", "15.3.1", "15.0.0"]
                .into_iter()
                .map(|v| ChartHit {
                    chart: ChartRef::new("nginx", v).in_repo("bitnami"),
                    app_version: Some("1.25.3".into()),
                    description: "NGINX".into(),
                })
                .collect(),
        );
        fake.script().show.push_ok(DEFAULTS_OLD.to_owned());
        fake.script().show.push_ok(DEFAULTS_NEW.to_owned());
        fake.script().list_releases.push_ok(vec![
            Release::summary(
                web(),
                ChartRef::new("nginx", "15.0.0"),
                3,
                ReleaseStatus::Deployed,
                Timestamp::UNIX_EPOCH,
            ),
            Release::summary(
                ReleaseRef::new(cluster(), "demo", "db"),
                ChartRef::new("postgresql", "13.0.0"),
                1,
                ReleaseStatus::Failed,
                Timestamp::UNIX_EPOCH,
            ),
        ]);

        let connector = Arc::new(FakeClusterConnectorPort::new());
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
            .install("oxikube_app::helm_upgrade", |r| {
                oxikube_app::helm_upgrade::register_commands(
                    r,
                    port,
                    follow,
                    UpgradeRequests::new(),
                )
            })
            .expect("registers");
        let guard = MutationGuard::new(sessions, state.clone(), clock.clone());
        let bus = CommandBus::new(registry, guard);
        let flow = UpgradeFlow::new(bus, fake.clone(), progress, "me");
        let repos = HelmReposService::new(fake.clone(), fake.clone(), clock);
        let charts = ChartsService::new(fake.clone(), repos.catalog_changes());

        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
        let deps = UpgradeDeps {
            backend: Rc::new(FlowBackend::new(flow, charts)),
            cluster: cluster(),
            cluster_label: "kind-oxikube".into(),
        };
        let mut view = None;
        vcx.update(|window, cx| {
            let entity = cx.new(|cx| HelmUpgradeView::new(deps, options.release, window, cx));
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

    /// Runs what is ready and lets the redraws (and the diffs' debounce) land.
    pub fn settle(&mut self) {
        for _ in 0..6 {
            self.vcx.run_until_parked();
            self.vcx
                .executor()
                .advance_clock(std::time::Duration::from_millis(200));
        }
        self.vcx.run_until_parked();
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&HelmUpgradeView, &gpui::App) -> R) -> R {
        let view = self.view.clone();
        self.vcx.read(|cx| f(view.read(cx), cx))
    }

    pub fn update<R>(
        &mut self,
        f: impl FnOnce(
            &mut HelmUpgradeView,
            &mut gpui::Window,
            &mut gpui::Context<HelmUpgradeView>,
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

    /// Replaces the values in the editor.
    pub fn set_values(&mut self, text: &str) {
        self.update(|view, window, cx| view.set_values(text, window, cx));
    }

    /// Scripts a passing preview: the current manifest and chart, the template, the server's dry
    /// run.
    pub fn script_preview(&self) {
        let script = self.fake.script();
        script.manifest.push_ok(CURRENT_MANIFEST.to_owned());
        script
            .history
            .push_ok(vec![revision(3, "15.0.0", ReleaseStatus::Deployed)]);
        script.template.push_ok(RENDERED.to_owned());
        script.upgrade.push_ok(Vec::new());
    }

    pub fn audit(&self) -> Vec<oxikube_domain::audit::AuditRecord> {
        self.state.audit_log()
    }
}
