//! [`DrainFlags`]: the choices a person (or an agent) makes for one drain, as `kubectl drain`
//! flags.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::options::DrainOptions;
use crate::{OxiError, OxiResult};

/// The longest grace period a drain accepts, in seconds (one day).
pub const MAX_GRACE_PERIOD_SECS: u32 = 86_400;

/// The longest timeout a drain accepts, in seconds (one day).
pub const MAX_TIMEOUT_SECS: u32 = 86_400;

/// The timeout a drain gets when none is chosen: five minutes.
pub const DEFAULT_TIMEOUT_SECS: u32 = 300;

/// What a drain may do, as the flags of `kubectl drain`: the payload of `node::Drain` and the
/// choices of the drain dialog.
///
/// [`Default`] is the dialog's starting point: DaemonSet pods ignored (a DaemonSet recreates them
/// at once, so draining one only fails the drain), no emptyDir data deleted, unmanaged pods not
/// forced, each pod's own grace period, five minutes in all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrainFlags {
    /// Skip DaemonSet-managed pods (`--ignore-daemonsets`); without it such a pod blocks the drain.
    pub ignore_daemonsets: bool,
    /// Evict pods using `emptyDir` volumes, whose data is lost (`--delete-emptydir-data`).
    pub delete_emptydir_data: bool,
    /// Evict pods no controller manages; nothing recreates them (`--force`).
    pub force: bool,
    /// Seconds each pod gets to stop (`--grace-period`); `None` uses the pod's own
    /// (`--grace-period=-1`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grace_period_secs: Option<u32>,
    /// How long the whole drain may take, in seconds (`--timeout`); a pod still blocked or still
    /// terminating then is reported as failed.
    pub timeout_secs: u32,
}

impl Default for DrainFlags {
    fn default() -> Self {
        Self {
            ignore_daemonsets: true,
            delete_emptydir_data: false,
            force: false,
            grace_period_secs: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
}

impl DrainFlags {
    /// Checks the numbers: a timeout of at least one second, both within a day.
    ///
    /// # Errors
    ///
    /// `Validation`, naming the field.
    pub fn validate(&self) -> OxiResult<()> {
        if self.timeout_secs == 0 || self.timeout_secs > MAX_TIMEOUT_SECS {
            return Err(OxiError::validation(format!(
                "the drain timeout must be between 1 and {MAX_TIMEOUT_SECS} seconds"
            )));
        }
        if self
            .grace_period_secs
            .is_some_and(|grace| grace > MAX_GRACE_PERIOD_SECS)
        {
            return Err(OxiError::validation(format!(
                "the grace period must be at most {MAX_GRACE_PERIOD_SECS} seconds"
            )));
        }
        Ok(())
    }

    /// The algorithm's options for these flags (retry and polling at their defaults).
    pub fn options(&self) -> DrainOptions {
        DrainOptions {
            ignore_daemonsets: self.ignore_daemonsets,
            delete_emptydir_data: self.delete_emptydir_data,
            force: self.force,
            grace_period_secs: self.grace_period_secs,
            timeout: Duration::from_secs(u64::from(self.timeout_secs)),
            ..DrainOptions::default()
        }
    }

    /// The `kubectl drain` command line this drain is the equivalent of, flags in kubectl's
    /// order, for the dialog (shown for transparency, never run).
    pub fn kubectl_command(&self, node: &str) -> String {
        let mut line = format!("kubectl drain {node}");
        for (on, flag) in [
            (self.ignore_daemonsets, "--ignore-daemonsets"),
            (self.delete_emptydir_data, "--delete-emptydir-data"),
            (self.force, "--force"),
        ] {
            if on {
                line.push(' ');
                line.push_str(flag);
            }
        }
        if let Some(grace) = self.grace_period_secs {
            line.push_str(&format!(" --grace-period={grace}"));
        }
        line.push_str(&format!(" --timeout={}s", self.timeout_secs));
        line
    }

    /// `ignore_daemonsets=true delete_emptydir_data=false force=false grace=pod timeout=300s`:
    /// the audit detail of a drain (flag names and numbers only).
    pub fn audit_words(&self) -> String {
        let grace = self
            .grace_period_secs
            .map_or_else(|| "pod".to_owned(), |grace| format!("{grace}s"));
        format!(
            "ignore_daemonsets={} delete_emptydir_data={} force={} grace={grace} timeout={}s",
            self.ignore_daemonsets, self.delete_emptydir_data, self.force, self.timeout_secs
        )
    }
}
