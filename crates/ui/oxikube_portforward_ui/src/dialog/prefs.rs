//! [`LastUsed`]: the bind address and local-port mode of the last start, remembered for the next
//! dialog. A per-window convenience kept in memory, not a setting and not on disk. A bind address
//! is remembered only when the user typed one other than `portforward.default_bind`, so the
//! setting (and its hot reload) keeps applying to every dialog until then.

use gpui::{App, Global};

/// What the next dialog starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastUsed {
    /// The bind address text the user chose over `portforward.default_bind`; `None` takes the
    /// setting.
    pub bind: Option<String>,
    /// Whether the local port was Auto.
    pub local_auto: bool,
}

impl Default for LastUsed {
    fn default() -> Self {
        Self {
            bind: None,
            local_auto: true,
        }
    }
}

impl Global for LastUsed {}

impl LastUsed {
    /// The remembered choices, or the defaults (the bind setting, Auto).
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }

    /// Remembers `bind` and `local_auto` for the next dialog.
    pub fn set(bind: Option<String>, local_auto: bool, cx: &mut App) {
        cx.set_global(Self { bind, local_auto });
    }
}
