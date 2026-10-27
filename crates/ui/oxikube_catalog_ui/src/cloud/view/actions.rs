//! What the Cloud section's buttons and keys do. Every one sends a `cloud::*` command through
//! the backend and shows what comes back.

use gpui::{ClipboardItem, Context};
use oxikube_app::command_bus::CommandOutput;
use oxikube_domain::cloud::{CloudProvider, CloudScope, DiscoveredCluster};
use oxikube_domain::command::{CloudRefreshMode, Command};
use oxikube_domain::{OxiError, OxiResult};

use super::{CloudView, Notice};
use crate::cloud::remediation::{Issue, remediation};

impl CloudView {
    /// Refreshes every provider (`cloud::Refresh` without a provider). Ignored while any
    /// provider is already running, so rapid clicks start one run.
    pub fn refresh_all(&mut self, mode: CloudRefreshMode, cx: &mut Context<Self>) {
        if self.model.any_running() {
            return;
        }
        for provider in CloudProvider::ALL {
            self.model.begin(provider);
        }
        self.notice = None;
        self.send_refresh(None, mode, cx);
    }

    /// Refreshes one provider (the user's "Refresh": the CLI runs even when the cache is fresh).
    /// Ignored while that provider is already running.
    pub fn refresh_provider(&mut self, provider: CloudProvider, cx: &mut Context<Self>) {
        if !self.model.begin(provider) {
            return;
        }
        self.notice = None;
        self.send_refresh(Some(provider), CloudRefreshMode::Force, cx);
    }

    fn send_refresh(
        &mut self,
        provider: Option<CloudProvider>,
        mode: CloudRefreshMode,
        cx: &mut Context<Self>,
    ) {
        let task = self
            .deps
            .backend
            .run(Command::CloudRefresh { provider, mode }, cx);
        // Detached: the run is stopped by `cloud::Cancel`, not by dropping a task, and the
        // task never clears a slot of its own.
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| this.finish_refresh(provider, result, cx))
                .ok();
        })
        .detach();
        cx.notify();
    }

    fn finish_refresh(
        &mut self,
        provider: Option<CloudProvider>,
        result: OxiResult<CommandOutput>,
        cx: &mut Context<Self>,
    ) {
        for provider in provider.map_or(CloudProvider::ALL.to_vec(), |p| vec![p]) {
            self.model.end(provider);
        }
        if let Err(error) = result {
            // The provider's own `Failed` event shows the remediation; this is for a run that
            // ended without one (the service is gone).
            tracing::debug!(%error, "a cloud refresh ended with an error");
        }
        cx.notify();
    }

    /// Stops every running discovery (`cloud::Cancel`), the Escape key. Does nothing when none
    /// runs.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if !self.model.any_running() {
            return;
        }
        self.deps
            .backend
            .run(Command::CloudCancel { provider: None }, cx)
            .detach();
    }

    /// Adds `cluster` to the catalog (`cloud::Add`).
    pub fn add(&mut self, cluster: &DiscoveredCluster, cx: &mut Context<Self>) {
        let command = Command::CloudAdd {
            provider: cluster.provider(),
            cluster: cluster.key(),
        };
        self.edit(command, cluster.provider(), Some(&cluster.scope), cx);
    }

    /// Asks for a second click to remove `cluster`: the first click arms it.
    pub fn arm_remove(&mut self, cluster: &DiscoveredCluster, cx: &mut Context<Self>) {
        self.confirm_remove = Some(cluster.key());
        cx.notify();
    }

    /// Withdraws the removal that waits for a second click.
    pub fn disarm_remove(&mut self, cx: &mut Context<Self>) {
        if self.confirm_remove.take().is_some() {
            cx.notify();
        }
    }

    /// Removes `cluster` from the catalog (`cloud::Remove`); an open session of it is
    /// disconnected by the service.
    pub fn remove(&mut self, cluster: &DiscoveredCluster, cx: &mut Context<Self>) {
        self.confirm_remove = None;
        let Some(context) = self.model.added_as(cluster).cloned() else {
            return;
        };
        let command = Command::CloudRemove {
            provider: cluster.provider(),
            context,
        };
        self.edit(command, cluster.provider(), Some(&cluster.scope), cx);
    }

    /// Runs an add or a remove, then shows its outcome and re-reads what is in the catalog.
    fn edit(
        &mut self,
        command: Command,
        provider: CloudProvider,
        scope: Option<&CloudScope>,
        cx: &mut Context<Self>,
    ) {
        if self.editing {
            return;
        }
        self.editing = true;
        let scope = scope.cloned();
        let task = self.deps.backend.run(command, cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.editing = false;
                this.show_outcome(provider, scope.as_ref(), &result, cx);
                this.reload_added(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn show_outcome(
        &mut self,
        provider: CloudProvider,
        scope: Option<&CloudScope>,
        result: &OxiResult<CommandOutput>,
        cx: &mut Context<Self>,
    ) {
        let notice = match result {
            Ok(output) => output.message.clone().map(|text| Notice {
                text: text.into(),
                command: None,
                error: false,
            }),
            Err(error) => Some(error_notice(provider, scope, error)),
        };
        self.notice = notice;
        cx.notify();
    }

    /// Re-reads which clusters are in the catalog. Local file work, off the UI thread.
    pub fn reload_added(&mut self, cx: &mut Context<Self>) {
        for provider in CloudProvider::ALL {
            let task = self.deps.backend.added(provider, cx);
            cx.spawn(async move |this, cx| {
                let Ok(entries) = task.await else {
                    return;
                };
                this.update(cx, |this, cx| {
                    this.model.set_added(provider, &entries);
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// Copies a fixing command to the clipboard: the stand-in for "Open terminal" until the
    /// terminal can be opened without a cluster.
    pub fn copy_command(&mut self, command: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(command.to_owned()));
        self.say(
            Notice {
                text: format!("Copied: {command}").into(),
                command: None,
                error: false,
            },
            cx,
        );
    }
}

/// A failed command as a notice: the remediation of its issue, else the error's own message.
fn error_notice(provider: CloudProvider, scope: Option<&CloudScope>, error: &OxiError) -> Notice {
    let issue = Issue::of(error);
    if issue == Issue::Other {
        return Notice {
            text: error.message().to_owned().into(),
            command: None,
            error: true,
        };
    }
    let fix = remediation(provider, issue, scope);
    Notice {
        text: fix.text.into(),
        command: fix.command,
        error: true,
    }
}
