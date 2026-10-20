//! [`LastUsed`]: the bind address and local-port mode of the last start, remembered for the next
//! dialog. A per-window convenience kept in memory, not a setting and not on disk.

use gpui::{App, Global};

/// What the next dialog starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastUsed {
    /// The bind address text.
    pub bind: String,
    /// Whether the local port was Auto.
    pub local_auto: bool,
}

impl Default for LastUsed {
    fn default() -> Self {
        Self {
            bind: oxikube_domain::portforward::ForwardSpec::DEFAULT_BIND.to_string(),
            local_auto: true,
        }
    }
}

impl Global for LastUsed {}

impl LastUsed {
    /// The remembered choices, or the defaults (loopback, Auto).
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }

    /// Remembers `bind` and `local_auto` for the next dialog.
    pub fn set(bind: String, local_auto: bool, cx: &mut App) {
        cx.set_global(Self { bind, local_auto });
    }
}
