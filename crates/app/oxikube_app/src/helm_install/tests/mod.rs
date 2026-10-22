//! `helm::Install` through the real guard over `FakeHelmPort`: preview, confirm, stream, cancel,
//! read-only, audit.

mod flow;
mod parts;

use std::sync::Arc;

use futures::FutureExt as _;
use oxikube_domain::command::CommandId;
use oxikube_domain::helm::{ChartRef, OutputLine, ValuesYaml};
use oxikube_testkit::{FakeHelmPort, HelmCall};

use super::{InstallFlow, InstallSpec, ReleaseProgress, register_commands};
use crate::testing::{Harness, id};

/// A rendered chart with a Service and a Secret.
pub(super) const RENDERED: &str = "---\n# Source: nginx/templates/svc.yaml\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n---\n# Source: nginx/templates/secret.yaml\napiVersion: v1\nkind: Secret\nmetadata:\n  name: web-auth\ndata:\n  password: c3VwZXJzZWNyZXQ=\n";

pub(super) struct Setup {
    pub h: Harness,
    pub helm: Arc<FakeHelmPort>,
    pub flow: InstallFlow,
}

impl Setup {
    /// The real handler on cluster `a` (connected, `read_only` as given).
    pub fn new(read_only: bool) -> Self {
        let helm = Arc::new(FakeHelmPort::new());
        let progress = ReleaseProgress::new();
        let (port, follow) = (helm.clone(), progress.clone());
        let h = Harness::build(&[], move |reg, _, _| register_commands(reg, port, follow));
        h.connect("a", read_only);
        let flow = InstallFlow::new(h.bus.clone(), h.manager.clone(), progress, "me");
        Self { h, helm, flow }
    }

    /// Scripts one preview: the template and a passing server dry run.
    pub fn script_preview(&self) {
        self.helm.script().template.push_ok(RENDERED.to_owned());
        self.helm.script().install.push_ok(Vec::new());
    }

    pub fn audit_of_installs(&self) -> Vec<oxikube_domain::audit::AuditRecord> {
        self.h
            .audit()
            .into_iter()
            .filter(|r| &*r.cmd == CommandId::HELM_INSTALL.as_str())
            .collect()
    }

    pub fn installs(&self) -> Vec<oxikube_domain::helm::InstallRequest> {
        self.helm
            .recorded_calls()
            .into_iter()
            .filter_map(|call| match call {
                HelmCall::Install(request) => Some(request),
                _ => None,
            })
            .collect()
    }
}

/// `bitnami/nginx 15.4.0` as `web` in `demo` on cluster `a`, with a password in its values.
pub(super) fn spec() -> InstallSpec {
    let mut spec = InstallSpec::new(
        id("a"),
        "web",
        "demo",
        ChartRef::new("nginx", "15.4.0").in_repo("bitnami"),
    );
    spec.create_namespace = true;
    spec.values = ValuesYaml::new("replicaCount: 2\nauth:\n  password: hunter2-value\n");
    spec
}

pub(super) fn line(text: &str) -> OutputLine {
    OutputLine::stdout(text)
}

/// Runs a future that the fakes answer at once.
pub(super) fn now<F: std::future::Future>(future: F) -> F::Output {
    future.now_or_never().expect("the fakes answer at once")
}
