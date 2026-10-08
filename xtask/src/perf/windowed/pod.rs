//! The pod the `terminal` scenario's shell runs in when kind is available: a busybox pod in a
//! namespace of its own (`oxi-perf-tty-<pid>`), deleted when the run is over. Every kubectl call
//! names the context; nothing outside that namespace is touched.

use anyhow::{Context as _, Result, bail};
use std::process::{Command, Output};

/// The image (from `fixtures/test-images.txt`, which `kind-up` pre-pulls).
const IMAGE: &str = "registry.k8s.io/e2e-test-images/busybox:1.36.1-1";
/// The pod's name.
const POD: &str = "tty";

/// A running shell pod; the namespace goes when this is dropped.
pub struct ShellPod {
    context: String,
    namespace: String,
}

impl ShellPod {
    /// `CONTEXT/NAMESPACE/POD` for `oxikube --perf-exec`.
    pub fn exec_arg(&self) -> String {
        format!("{}/{}/{POD}", self.context, self.namespace)
    }

    /// Creates the namespace and the pod on `context` and waits until it runs. `Ok(None)` when the
    /// context does not answer (no kind cluster): the scenario then uses a local shell.
    pub fn start(context: &str) -> Result<Option<Self>> {
        if !kubectl(
            context,
            &["get", "--raw", "/readyz", "--request-timeout=5s"],
        )
        .is_ok_and(|o| o.status.success())
        {
            return Ok(None);
        }
        let namespace = format!("oxi-perf-tty-{}", std::process::id());
        let pod = Self {
            context: context.to_owned(),
            namespace: namespace.clone(),
        };
        ok(kubectl(context, &["create", "namespace", &namespace])?)
            .context("creating the shell pod's namespace")?;
        ok(kubectl(
            context,
            &[
                "-n",
                &namespace,
                "run",
                POD,
                "--image",
                IMAGE,
                "--restart=Never",
                "--command",
                "--",
                "sleep",
                "3600",
            ],
        )?)
        .context("creating the shell pod")?;
        ok(kubectl(
            context,
            &[
                "-n",
                &namespace,
                "wait",
                "--for=condition=Ready",
                &format!("pod/{POD}"),
                "--timeout=120s",
            ],
        )?)
        .context("waiting for the shell pod")?;
        Ok(Some(pod))
    }
}

impl Drop for ShellPod {
    fn drop(&mut self) {
        let _ = kubectl(
            &self.context,
            &["delete", "namespace", &self.namespace, "--wait=false"],
        );
    }
}

fn kubectl(context: &str, args: &[&str]) -> Result<Output> {
    Command::new("kubectl")
        .arg("--context")
        .arg(context)
        .args(args)
        .output()
        .context("running kubectl")
}

fn ok(output: Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    bail!("{}", String::from_utf8_lossy(&output.stderr).trim())
}
