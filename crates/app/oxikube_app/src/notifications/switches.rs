//! What the user can switch: the muted clusters (remembered on disk) and the kinds of
//! notification that are on. Methods of [`NotificationService`].

use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::notification::NotificationKind;

use super::driver;
use super::mute;
use super::service::NotificationService;

impl NotificationService {
    /// Switches `kind` on or off: while off, nothing of that kind is added (the rows already
    /// listed stay). Every kind is on by default; the settings (E14-S09) decide at start-up.
    /// Returns whether the setting changed.
    pub fn set_kind_enabled(&self, kind: NotificationKind, enabled: bool) -> bool {
        self.change(|state| {
            if enabled {
                state.disabled_kinds.remove(&kind)
            } else {
                state.disabled_kinds.insert(kind)
            }
        })
    }

    /// Whether notifications of `kind` are added.
    pub fn is_kind_enabled(&self, kind: NotificationKind) -> bool {
        !self.inner.state.lock().disabled_kinds.contains(&kind)
    }

    /// Whether `cluster` is muted.
    pub fn is_muted(&self, cluster: &ClusterId) -> bool {
        self.inner.state.lock().muted.contains(cluster)
    }

    /// The muted clusters.
    pub fn muted(&self) -> Vec<ClusterId> {
        self.inner.state.lock().muted.iter().cloned().collect()
    }

    /// Mutes `cluster`: its events stop being watched for notifications (its feed may still run
    /// for the views) and nothing about it is added; the rows already listed stay. Remembered
    /// across restarts. Returns whether it was unmuted.
    ///
    /// # Errors
    ///
    /// The state store's error when the change could not be saved (it still applies until the
    /// app quits).
    pub async fn mute(&self, cluster: &ClusterId) -> OxiResult<bool> {
        let changed = self.change(|state| {
            let changed = state.muted.insert(cluster.clone());
            if changed {
                state.watches.remove(cluster);
                state.limiter.forget(cluster);
            }
            changed
        });
        if changed {
            self.persist().await?;
        }
        Ok(changed)
    }

    /// Unmutes `cluster`: notifying resumes with what happens from now on. Returns whether it
    /// was muted.
    ///
    /// # Errors
    ///
    /// As for [`mute`](Self::mute).
    pub async fn unmute(&self, cluster: &ClusterId) -> OxiResult<bool> {
        let changed = self.change(|state| state.muted.remove(cluster));
        if changed {
            driver::reconcile(&self.inner, cluster);
            self.persist().await?;
        }
        Ok(changed)
    }

    pub(super) async fn persist(&self) -> OxiResult<()> {
        let _in_order = self.inner.persist.lock().await;
        let muted = self.inner.state.lock().muted.clone();
        mute::save(&*self.inner.store, &muted).await
    }
}
