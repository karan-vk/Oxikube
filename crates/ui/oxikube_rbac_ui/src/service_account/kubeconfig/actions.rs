//! What the section's buttons do.

use std::path::PathBuf;
use std::time::Duration;

use gpui::{ClipboardItem, Context};
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_runtime::spawn_kube;

use super::state::{Expiry, deps};
use crate::service_account::panel::ServiceAccountPanel;

/// How long a request is waited for before the section stops saying "Requesting…" (the bus shows
/// the failure itself).
pub(super) const WAIT_LIMIT: Duration = Duration::from_secs(30);

impl ServiceAccountPanel {
    /// Chooses the token's lifetime.
    pub fn set_kubeconfig_expiry(&mut self, expiry: Expiry, cx: &mut Context<Self>) {
        self.kubeconfig.expiry = expiry;
        cx.notify();
    }

    /// Allows (or not) a legacy token Secret on a cluster without `TokenRequest`.
    pub fn set_kubeconfig_allow_legacy(&mut self, allow: bool, cx: &mut Context<Self>) {
        self.kubeconfig.allow_legacy = allow;
        cx.notify();
    }

    /// Shows or masks the token in the preview.
    pub fn toggle_kubeconfig_reveal(&mut self, cx: &mut Context<Self>) {
        self.kubeconfig.reveal = !self.kubeconfig.reveal;
        cx.notify();
    }

    /// Asks for a kubeconfig: sends `serviceaccount::GenerateKubeconfig` through the bus, whose
    /// guard confirms, audits and refuses on a read-only cluster. The result arrives in the
    /// vault and is picked up by the panel's next look.
    pub fn generate_kubeconfig(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.target(cx) else {
            return;
        };
        let vault = deps(cx).map_or_else(
            oxikube_app::sa_kubeconfig::KubeconfigVault::shared,
            |deps| deps.vault,
        );
        self.kubeconfig.asked_for(target.clone(), vault);
        self.kubeconfig.waiting = true;
        self.kubeconfig.waiting_since = Some(std::time::Instant::now());
        self.kubeconfig.note = None;
        let command = Command::ServiceAccountGenerateKubeconfig {
            target,
            expiration_seconds: Some(self.kubeconfig.expiry.seconds()),
            audiences: Vec::new(),
            allow_legacy_token: self.kubeconfig.allow_legacy,
        };
        self.send(command, cx);
        cx.notify();
    }

    /// Shows `generated` as if the vault had delivered it. For tests and screenshots, whose
    /// fixed panels are not polled and have no bus.
    pub fn show_kubeconfig(
        &mut self,
        generated: oxikube_app::sa_kubeconfig::GeneratedKubeconfig,
        cx: &mut Context<Self>,
    ) {
        self.kubeconfig.generated = Some(generated);
        cx.notify();
    }

    /// Puts the kubeconfig, token included, on the clipboard.
    pub fn copy_kubeconfig(&mut self, cx: &mut Context<Self>) {
        let Some(generated) = &self.kubeconfig.generated else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(
            generated.text.expose().to_owned(),
        ));
        self.kubeconfig.note = Some("Copied, token included.".into());
        cx.notify();
    }

    /// Asks where to save the kubeconfig and writes it there, readable by its owner only.
    pub fn save_kubeconfig(&mut self, cx: &mut Context<Self>) {
        let (Some(generated), Some(deps)) = (self.kubeconfig.generated.clone(), deps(cx)) else {
            self.kubeconfig.note = Some("Saving is not available here.".into());
            cx.notify();
            return;
        };
        let answer = cx.prompt_for_new_path(&save_directory(), Some(&generated.file_name()));
        self.kubeconfig_save = Some(cx.spawn(async move |this, cx| {
            // Cancelled, or the platform could not show the panel: nothing to write.
            let Ok(Ok(Some(path))) = answer.await else {
                return;
            };
            let fs = deps.fs.clone();
            let target = path.clone();
            let written = this.update(cx, |_, cx| {
                spawn_kube(cx, async move {
                    fs.write_private(&target, generated.text.expose().as_bytes())
                        .await
                })
            });
            let Ok(written) = written else {
                return;
            };
            let result = match written.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |panel, cx| {
                panel.kubeconfig.note = Some(match result {
                    Ok(()) => format!("Saved to {} (owner-only).", path.display()).into(),
                    Err(error) => {
                        tracing::warn!(error = %error.message(), "saving the kubeconfig failed");
                        format!("Could not save: {}", error.message()).into()
                    }
                });
                cx.notify();
            })
            .ok();
        }));
    }

    /// Drops the kubeconfig and its token.
    pub fn discard_kubeconfig(&mut self, cx: &mut Context<Self>) {
        self.kubeconfig.discard();
        self.kubeconfig.note = None;
        cx.notify();
    }

    /// [`refresh_kubeconfig`](Self::refresh_kubeconfig) for tests, whose fixed panels are not
    /// polled.
    #[cfg(test)]
    pub(in crate::service_account) fn refresh_kubeconfig_for_test(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        self.refresh_kubeconfig(cx);
    }

    /// Picks up the vault's kubeconfig for this account when the vault changed, and gives up
    /// waiting after [`WAIT_LIMIT`]. Returns whether the section is still waiting.
    pub(in crate::service_account) fn refresh_kubeconfig(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(deps) = deps(cx) else {
            return false;
        };
        let version = deps.vault.version();
        let mut changed = false;
        if version != self.kubeconfig.seen {
            self.kubeconfig.seen = version;
            if let Some(target) = self.target(cx) {
                let held = deps.vault.get(&target);
                if held.is_some() {
                    self.kubeconfig.waiting = false;
                    self.kubeconfig.note = None;
                }
                if held.is_some() || self.kubeconfig.generated.is_some() {
                    self.kubeconfig.generated = held;
                    changed = true;
                }
            }
        }
        if self.kubeconfig.waiting
            && self
                .kubeconfig
                .waiting_since
                .is_some_and(|since| since.elapsed() > WAIT_LIMIT)
        {
            self.kubeconfig.waiting = false;
            changed = true;
        }
        if changed {
            cx.notify();
        }
        self.kubeconfig.waiting
    }
}

/// Where the save panel opens: the user's home directory, else the current one.
fn save_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}
