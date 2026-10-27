//! What the kubeconfig section remembers: the choices, the result and what to say about it.

use std::sync::Arc;
use std::time::Instant;

use gpui::{App, Global, SharedString};
use oxikube_app::sa_kubeconfig::{GeneratedKubeconfig, KubeconfigVault};
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::FsPort;

/// How long the token should live. The server may shorten it and says what it chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    /// 10 minutes: the server's minimum.
    TenMinutes,
    /// 1 hour: the default.
    OneHour,
    /// 8 hours: a working day.
    EightHours,
    /// 1 day.
    OneDay,
    /// 7 days.
    SevenDays,
}

impl Expiry {
    /// Every choice, shortest first.
    pub const ALL: [Expiry; 5] = [
        Expiry::TenMinutes,
        Expiry::OneHour,
        Expiry::EightHours,
        Expiry::OneDay,
        Expiry::SevenDays,
    ];

    /// The lifetime in seconds.
    pub const fn seconds(self) -> u32 {
        match self {
            Expiry::TenMinutes => 600,
            Expiry::OneHour => 3_600,
            Expiry::EightHours => 28_800,
            Expiry::OneDay => 86_400,
            Expiry::SevenDays => 604_800,
        }
    }

    /// The button's label.
    pub const fn label(self) -> &'static str {
        match self {
            Expiry::TenMinutes => "10 min",
            Expiry::OneHour => "1 hour",
            Expiry::EightHours => "8 hours",
            Expiry::OneDay => "1 day",
            Expiry::SevenDays => "7 days",
        }
    }
}

/// What the section needs from the app: the filesystem Save writes through and the vault the
/// result is read from. Installed once by the binary ([`install`](crate::install)).
#[derive(Clone)]
pub struct KubeconfigDeps {
    /// Where Save writes (`write_private`: mode 0600).
    pub fs: Arc<dyn FsPort>,
    /// Where the handler leaves the result.
    pub vault: KubeconfigVault,
}

impl Global for KubeconfigDeps {}

/// The installed dependencies, if any.
pub fn deps(cx: &App) -> Option<KubeconfigDeps> {
    cx.try_global::<KubeconfigDeps>().cloned()
}

/// The section's state, one per panel. Dropping it (the panel closed) discards the kubeconfig the
/// panel asked for, so the token does not outlive the view that showed it.
pub struct KubeconfigState {
    /// The lifetime asked for.
    pub expiry: Expiry,
    /// Whether a legacy token Secret may be read or made on a cluster without `TokenRequest`.
    pub allow_legacy: bool,
    /// Whether the token is shown in the preview.
    pub reveal: bool,
    /// A request was sent and its result is not here yet.
    pub waiting: bool,
    /// When that request was sent.
    pub waiting_since: Option<Instant>,
    /// The result for the panel's account.
    pub generated: Option<GeneratedKubeconfig>,
    /// What the section last said about copying or saving.
    pub note: Option<SharedString>,
    /// The vault version last looked at.
    pub seen: u64,
    /// The account this panel asked a kubeconfig for, and the vault it will be in.
    asked: Option<(ResourceRef, KubeconfigVault)>,
}

impl Default for KubeconfigState {
    fn default() -> Self {
        Self {
            expiry: Expiry::OneHour,
            allow_legacy: false,
            reveal: false,
            waiting: false,
            waiting_since: None,
            generated: None,
            note: None,
            seen: u64::MAX,
            asked: None,
        }
    }
}

impl KubeconfigState {
    /// Remembers that `account`'s kubeconfig was asked for, so it is discarded with the panel.
    pub fn asked_for(&mut self, account: ResourceRef, vault: KubeconfigVault) {
        self.asked = Some((account, vault));
    }

    /// Drops the result, here and in the vault.
    pub fn discard(&mut self) {
        if let Some((account, vault)) = &self.asked {
            vault.discard(account);
        }
        self.generated = None;
        self.reveal = false;
        self.waiting = false;
    }
}

impl Drop for KubeconfigState {
    fn drop(&mut self) {
        if let Some((account, vault)) = &self.asked {
            vault.discard(account);
        }
    }
}
