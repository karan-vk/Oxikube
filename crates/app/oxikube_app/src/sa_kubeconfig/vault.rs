//! [`KubeconfigVault`]: where a generated kubeconfig waits, in memory, for the panel that asked.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use oxikube_domain::ids::ResourceRef;
use oxikube_domain::secret::SecretText;
use parking_lot::Mutex;

/// The most kubeconfigs held at once; the oldest is dropped past it.
const CAPACITY: usize = 8;

/// Where the token came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    /// A `TokenRequest`: short-lived, bound to the lifetime asked for.
    TokenRequest,
    /// A `kubernetes.io/service-account-token` Secret that already existed.
    LegacySecret,
    /// A Secret of that type made for this request.
    CreatedLegacySecret,
}

impl TokenSource {
    /// Whether the credential never expires on its own.
    pub fn is_long_lived(self) -> bool {
        !matches!(self, TokenSource::TokenRequest)
    }

    /// A short label for the dialog and the command's output.
    pub fn label(self) -> &'static str {
        match self {
            TokenSource::TokenRequest => "token_request",
            TokenSource::LegacySecret => "legacy_secret",
            TokenSource::CreatedLegacySecret => "created_legacy_secret",
        }
    }
}

/// One generated kubeconfig. `Debug` shows no token.
#[derive(Clone)]
pub struct GeneratedKubeconfig {
    /// The ServiceAccount.
    pub service_account: ResourceRef,
    /// The kubeconfig with the token in it: what Copy and Save use.
    pub text: SecretText,
    /// The same with the token masked: what the dialog shows by default.
    pub preview: String,
    /// Where the token came from.
    pub source: TokenSource,
    /// When the token expires, as the server said (`expirationTimestamp`); `None` for a legacy
    /// token, which does not.
    pub expires_at: Option<String>,
    /// A warning to show next to the file (a long-lived credential), if any.
    pub warning: Option<String>,
}

impl GeneratedKubeconfig {
    /// A file name to suggest when saving: `<namespace>-<account>-kubeconfig.yaml`.
    pub fn file_name(&self) -> String {
        format!(
            "{}-{}-kubeconfig.yaml",
            self.service_account.namespace().unwrap_or("default"),
            self.service_account.name
        )
    }
}

impl fmt::Debug for GeneratedKubeconfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedKubeconfig")
            .field("service_account", &self.service_account)
            .field("source", &self.source)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Held {
    /// Oldest first.
    entries: Vec<(ResourceRef, GeneratedKubeconfig)>,
}

/// The kubeconfigs generated and not yet discarded, by ServiceAccount. Cheap to clone: clones
/// share the store.
///
/// The `serviceaccount::GenerateKubeconfig` handler puts the result here and the command's
/// output says only that it is ready; the ServiceAccount panel takes it from here. The token
/// therefore never travels through the command bus, the audit log, a toast or an agent's tool
/// result. Nothing here is written to disk; [`discard`](Self::discard) (the panel's, when it
/// closes) drops the text, which zeroes the token's memory.
#[derive(Clone, Default)]
pub struct KubeconfigVault {
    held: Arc<Mutex<Held>>,
    version: Arc<AtomicU64>,
}

impl KubeconfigVault {
    /// An empty vault.
    pub fn new() -> Self {
        Self::default()
    }

    /// The process's vault: the one the app's handler fills and the panels of every window read,
    /// so a kubeconfig generated through one window's bus shows in the account's panel in
    /// another. Tests build their own with [`new`](Self::new).
    pub fn shared() -> Self {
        static SHARED: OnceLock<KubeconfigVault> = OnceLock::new();
        SHARED.get_or_init(Self::new).clone()
    }

    /// Keeps `kubeconfig` for its account, replacing an earlier one.
    pub fn put(&self, kubeconfig: GeneratedKubeconfig) {
        let mut held = self.held.lock();
        let account = kubeconfig.service_account.clone();
        held.entries.retain(|(key, _)| key != &account);
        held.entries.push((account, kubeconfig));
        if held.entries.len() > CAPACITY {
            held.entries.remove(0);
        }
        self.version.fetch_add(1, Ordering::Release);
    }

    /// The kubeconfig for `account`, if one is held.
    pub fn get(&self, account: &ResourceRef) -> Option<GeneratedKubeconfig> {
        self.held
            .lock()
            .entries
            .iter()
            .find(|(key, _)| key == account)
            .map(|(_, kubeconfig)| kubeconfig.clone())
    }

    /// Drops the kubeconfig for `account`.
    pub fn discard(&self, account: &ResourceRef) {
        let mut held = self.held.lock();
        let before = held.entries.len();
        held.entries.retain(|(key, _)| key != account);
        if held.entries.len() != before {
            self.version.fetch_add(1, Ordering::Release);
        }
    }

    /// A number that changes whenever something is put or discarded: a view compares it to its
    /// last look instead of reading the store.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// How many kubeconfigs are held.
    pub fn len(&self) -> usize {
        self.held.lock().entries.len()
    }

    /// Whether none is held.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for KubeconfigVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KubeconfigVault")
            .field("held", &self.len())
            .finish()
    }
}
