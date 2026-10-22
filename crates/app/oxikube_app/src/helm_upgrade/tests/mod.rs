//! `helm::Upgrade` through the real guard over `FakeHelmPort`: preview, confirm, stream, cancel,
//! read-only, audit.

mod flow;
mod native;
mod parts;

use std::sync::Arc;

use futures::FutureExt as _;
use jiff::Timestamp;
use oxikube_domain::command::CommandId;
use oxikube_domain::helm::{
    ChartRef, OutputLine, ReleaseRef, ReleaseStatus, Revision, UpgradeRequest, Values, ValuesYaml,
};
use oxikube_testkit::{FakeHelmPort, HelmCall};

use super::{UpgradeFlow, UpgradeRequests, UpgradeSpec, register_commands};
use crate::helm_install::ReleaseProgress;
use crate::testing::{Harness, id};

/// What the release runs now: a Service, a Deployment and a Secret.
pub(super) const CURRENT: &str = "---\n# Source: nginx/templates/svc.yaml\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 1\n---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-auth\ndata:\n  password: b2xkcGFzcw==\n";

/// What the new chart renders: the Deployment changed, the Service the same, the Secret gone, an
/// Ingress added.
pub(super) const RENDERED: &str = "---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n---\napiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\nspec:\n  replicas: 3\n---\napiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: web\n---\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-extra\ndata:\n  token: c3VwZXJzZWNyZXQ=\n";

pub(super) struct Setup {
    pub h: Harness,
    pub helm: Arc<FakeHelmPort>,
    pub flow: UpgradeFlow,
    pub requests: UpgradeRequests,
}

impl Setup {
    /// The real handler on cluster `a` (connected, `read_only` as given).
    pub fn new(read_only: bool) -> Self {
        let helm = Arc::new(FakeHelmPort::new());
        let progress = ReleaseProgress::new();
        let requests = UpgradeRequests::new();
        let (port, follow, asked) = (helm.clone(), progress.clone(), requests.clone());
        let h = Harness::build(&[], move |reg, _, _| {
            register_commands(reg, port, follow, asked)
        });
        h.connect("a", read_only);
        let flow = UpgradeFlow::new(h.bus.clone(), helm.clone(), progress, "me");
        Self {
            h,
            helm,
            flow,
            requests,
        }
    }

    /// Scripts one preview: the current manifest and chart, the template and a passing server
    /// dry run.
    pub fn script_preview(&self) {
        let script = self.helm.script();
        script.manifest.push_ok(CURRENT.to_owned());
        script.history.push_ok(vec![revision(3, "15.0.0")]);
        script.template.push_ok(RENDERED.to_owned());
        script.upgrade.push_ok(Vec::new());
    }

    pub fn audit_of_upgrades(&self) -> Vec<oxikube_domain::audit::AuditRecord> {
        self.h
            .audit()
            .into_iter()
            .filter(|r| &*r.cmd == CommandId::HELM_UPGRADE.as_str())
            .collect()
    }

    pub fn upgrades(&self) -> Vec<UpgradeRequest> {
        self.helm
            .recorded_calls()
            .into_iter()
            .filter_map(|call| match call {
                HelmCall::Upgrade(request) => Some(request),
                _ => None,
            })
            .collect()
    }
}

pub(super) fn revision(number: u32, version: &str) -> Revision {
    Revision {
        revision: number,
        status: ReleaseStatus::Deployed,
        chart: ChartRef::new("nginx", version),
        app_version: Some("1.25".into()),
        updated: Timestamp::UNIX_EPOCH,
        description: "Upgrade complete".into(),
    }
}

pub(super) fn web() -> ReleaseRef {
    ReleaseRef::new(id("a"), "demo", "web")
}

/// `bitnami/nginx 15.4.0` over `demo/web`, with a password in its values.
pub(super) fn spec() -> UpgradeSpec {
    let mut spec = UpgradeSpec::new(web(), ChartRef::new("nginx", "15.4.0").in_repo("bitnami"));
    spec.values = ValuesYaml::new("replicaCount: 3\nauth:\n  password: hunter2-value\n");
    spec
}

pub(super) fn line(text: &str) -> OutputLine {
    OutputLine::stdout(text)
}

pub(super) fn values(text: &str) -> Values {
    Values::from_yaml(text).unwrap()
}

/// Runs a future that the fakes answer at once.
pub(super) fn now<F: std::future::Future>(future: F) -> F::Output {
    future.now_or_never().expect("the fakes answer at once")
}
