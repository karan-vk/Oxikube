//! [`HelmGateState`]: the one place the Helm views ask what is available (E16-S11).
//!
//! The app detects the `helm` binary off the UI thread (`oxikube_helm::HelmCli`), then hands the
//! answer to [`set_cli_capabilities`](HelmGateState::set_cli_capabilities). Every Helm view, the
//! guidance banner, the palette and the tools read the resulting [`HelmGate`] from here instead
//! of checking `cli_available` themselves, so "hidden when absent" is decided once. Reads are
//! field reads: nothing here runs a process or touches the cluster, so rendering stays cheap.
//!
//! The state is an entity so views `observe` it: it notifies once per real change, never for a
//! recheck that found the same thing, and a recheck in flight only flips the
//! [`is_checking`](HelmGateState::is_checking) flag.

use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global};
use oxikube_domain::helm::{HelmCapabilities, HelmFeature, HelmGate};

/// Emitted when the gate (or the capabilities behind it) changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelmGateChanged;

/// What the Helm views may offer. See the [module docs](self).
#[derive(Debug, Default)]
pub struct HelmGateState {
    /// What the `helm` binary gave (never `native_read`).
    cli: HelmCapabilities,
    /// Whether a native release reader is mounted (E16-S02, S05).
    native_read: bool,
    /// The gate for `cli` and `native_read`.
    gate: HelmGate,
    /// A detection has reported at least once (before that the gate is `NotConfigured`).
    detected: bool,
    /// A detection is running.
    checking: bool,
}

impl EventEmitter<HelmGateChanged> for HelmGateState {}

/// The window-independent handle, so a view finds the state without being given it.
struct GlobalGate(Entity<HelmGateState>);

impl Global for GlobalGate {}

impl HelmGateState {
    /// The app's gate state, created on first use (nothing detected yet:
    /// [`Unavailable`](HelmGate::Unavailable) with `NotConfigured`).
    pub fn install(cx: &mut App) -> Entity<Self> {
        if let Some(existing) = Self::global(cx) {
            return existing;
        }
        let state = cx.new(|_| Self::default());
        cx.set_global(GlobalGate(state.clone()));
        state
    }

    /// The app's gate state, `None` before [`install`](Self::install).
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<GlobalGate>().map(|g| g.0.clone())
    }

    /// The gate in effect: the app's, or `Unavailable` without one.
    pub fn current(cx: &App) -> HelmGate {
        Self::global(cx).map_or_else(HelmGate::default, |state| state.read(cx).gate.clone())
    }

    /// The gate.
    pub fn gate(&self) -> &HelmGate {
        &self.gate
    }

    /// Whether `feature` may be offered now.
    pub fn allows(&self, feature: HelmFeature) -> bool {
        self.gate.allows(feature)
    }

    /// The combined capabilities (the binary's plus the native reader's).
    pub fn capabilities(&self) -> HelmCapabilities {
        HelmCapabilities {
            native_read: self.native_read || self.cli.native_read,
            ..self.cli.clone()
        }
    }

    /// Whether a detection is running.
    pub fn is_checking(&self) -> bool {
        self.checking
    }

    /// Records what detection found, ends the check, and notifies when the gate changed.
    pub fn set_cli_capabilities(&mut self, cli: HelmCapabilities, cx: &mut Context<Self>) {
        let was_checking = std::mem::replace(&mut self.checking, false);
        if self.detected && self.cli == cli {
            // Same answer: only a visible "checking" state ended.
            if was_checking {
                cx.notify();
            }
            return;
        }
        self.cli = cli;
        self.detected = true;
        self.refresh(cx);
        cx.notify();
    }

    /// Records whether a native release reader is mounted (releases can then be read with no
    /// binary).
    pub fn set_native_read(&mut self, native_read: bool, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.native_read, native_read) != native_read {
            self.refresh(cx);
            cx.notify();
        }
    }

    /// Shows a detection in flight (the banner's "Checking..."); the next
    /// [`set_cli_capabilities`](Self::set_cli_capabilities) ends it.
    pub fn begin_check(&mut self, cx: &mut Context<Self>) {
        if !std::mem::replace(&mut self.checking, true) {
            cx.notify();
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.detected {
            return;
        }
        let gate = HelmGate::from_capabilities(&self.capabilities());
        if gate != self.gate {
            self.gate = gate;
            cx.emit(HelmGateChanged);
        }
    }
}
