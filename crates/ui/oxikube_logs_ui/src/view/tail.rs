//! Tail in terminal (`shift-t`, `logs::TailInTerminal`): `kubectl logs -f` for what the view shows,
//! in a terminal tab of the cluster.
//!
//! The safety net and the power-user escape hatch: the viewer does not cover every `kubectl logs`
//! behaviour, and some users want kubectl exactly as they know it. The command line is built from
//! the view's own options ([`KubectlTail`]: container, previous instance, range, timestamps; a
//! workload's pods by their selector with `--prefix`), run as an argv, never through a shell. The
//! terminal is a normal local terminal of the cluster: its kubeconfig, context and namespace come
//! through the environment exactly as for `terminal::New` (the descriptor holds none of it), and
//! nothing on its screen is saved.
//!
//! The action is hidden when kubectl is not installed ([`LogView::can_tail_in_terminal`], from
//! the app's cached [`Kubectl`](oxikube_app::logs::kubectl::Kubectl) lookup, never a process
//! started on the UI thread). Nothing here touches the cluster: `kubectl logs` only reads.

use gpui::{App, Context};
use oxikube_app::logs::kubectl::{KubectlTail, TailTarget};
use oxikube_domain::command::Command;
use oxikube_terminal::view::BackendDescriptor;
use oxikube_workspace::Toast;

use super::LogView;
use crate::LogsSettings;

impl LogView {
    /// Whether the toolbar offers "Tail in terminal": kubectl was found on this machine.
    pub fn can_tail_in_terminal(&self) -> bool {
        self.deps.kubectl.is_available()
    }

    /// Asks to tail the log in a terminal (`logs::TailInTerminal`).
    pub fn request_tail_in_terminal(&mut self, cx: &mut Context<Self>) {
        let target = self.target.clone();
        self.send(Command::LogsTailInTerminal { target }, cx);
    }

    /// Opens a terminal tab of the view's cluster running `kubectl logs -f` for what the view
    /// shows. A toast says why when it cannot (no kubectl, a workload whose selector is not read
    /// yet, a cluster that is not open).
    pub fn tail_in_terminal(&mut self, cx: &mut Context<Self>) {
        let toast = match self.tail_descriptor(cx) {
            Ok(descriptor) => {
                if self.deps.terminal.open(descriptor) {
                    return;
                }
                Toast::error("The terminal could not be opened: its window is gone.")
            }
            Err(why) => Toast::info(why),
        };
        self.toast(toast.key("logs-tail-in-terminal"), cx);
    }

    /// The terminal that tails what the view shows: kubectl with its arguments, in the view's
    /// cluster and namespace, titled `logs <name>`. The cluster's environment is added when the
    /// terminal starts (`terminal::LocalLauncher`).
    ///
    /// # Errors
    ///
    /// Why there is nothing to run, in words for a toast.
    pub fn tail_descriptor(&self, cx: &App) -> Result<BackendDescriptor, String> {
        let Some(kubectl) = self.deps.kubectl.path() else {
            return Err("kubectl was not found on this machine.".to_owned());
        };
        let Some(session) = self.deps.sessions.get(&self.target.cluster) else {
            return Err("The cluster is not open.".to_owned());
        };
        let Some(namespace) = self.target.namespace.as_deref() else {
            return Err("Only a namespaced object has logs to tail.".to_owned());
        };
        let target = match &self.aggregate {
            None => TailTarget::Pod(self.target.name.to_string()),
            Some(aggregate) => aggregate
                .session
                .as_ref()
                .and_then(|session| session.selector())
                .map(TailTarget::Selector)
                .ok_or_else(|| {
                    "The pods of this view are still being looked up; try again in a moment."
                        .to_owned()
                })?,
        };
        let request = KubectlTail {
            context: session.context().as_str().to_owned(),
            namespace: namespace.to_owned(),
            target,
            options: self.options.log_options(),
            timestamps: self.options.timestamps,
            max_log_requests: LogsSettings::resolve(&self.target.cluster, cx).max_streams,
        };
        let args = request.argv().map_err(|error| error.to_string())?;
        let name = match &self.options.container {
            Some(container) if self.aggregate.is_none() => {
                format!("{}/{container}", self.subject())
            }
            _ => self.subject(),
        };
        Ok(BackendDescriptor::local(Some(self.target.cluster.clone()))
            .in_namespace(Some(namespace.to_owned()))
            .with_shell(kubectl.to_string_lossy().into_owned(), args)
            .titled(format!("logs {name}")))
    }
}
