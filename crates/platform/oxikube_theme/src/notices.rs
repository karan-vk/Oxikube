//! Things the theme layer wants the user to hear about, announced as events.
//!
//! This crate is platform code and cannot show a toast, so it announces and the binary shows:
//! [`subscribe_notices`] hands the binary a [`ThemeNotice`]. One channel serves every notice of
//! this epic (an unknown theme name today; broken theme files and the like later).
//!
//! An unknown theme name is announced **once per name**: re-resolving after an unrelated edit,
//! a rescan or a system appearance flip raises nothing, while fixing the name (or installing the
//! theme) and breaking it again announces anew. Notices raised before anyone subscribed are not
//! replayed; read [`pending_unknown_theme`] right after subscribing.

use gpui::{App, AppContext as _, Entity, EventEmitter, Global, Subscription};
use std::collections::BTreeSet;

/// One thing the user should be told about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThemeNotice {
    /// The `theme` setting names a theme that is not installed; a default theme is shown instead.
    UnknownTheme {
        /// The name the setting asked for.
        name: String,
        /// The name of the theme shown instead.
        fallback: String,
    },
}

impl ThemeNotice {
    /// The sentence for a toast.
    pub fn message(&self) -> String {
        match self {
            Self::UnknownTheme { name, fallback } => {
                format!("Theme \"{name}\" is not installed; showing \"{fallback}\" instead.")
            }
        }
    }
}

/// The entity notices are emitted from.
struct Emitter;

impl EventEmitter<ThemeNotice> for Emitter {}

struct Notices {
    emitter: Entity<Emitter>,
    /// Names announced and still unknown.
    reported: BTreeSet<String>,
    /// The latest unknown-name notice not yet cleared, for subscribers that arrive late.
    pending: Option<ThemeNotice>,
    /// Whether the first scan of the user themes is in. Before it, a missing name may only be a
    /// file that has not been read yet.
    scanned: bool,
}

impl Global for Notices {}

fn notices(cx: &mut App) -> &mut Notices {
    if !cx.has_global::<Notices>() {
        let emitter = cx.new(|_| Emitter);
        cx.set_global(Notices {
            emitter,
            reported: BTreeSet::new(),
            pending: None,
            scanned: false,
        });
    }
    cx.global_mut::<Notices>()
}

/// Runs `on_notice` for every [`ThemeNotice`]. Keep the subscription as long as it should run.
pub fn subscribe_notices(
    cx: &mut App,
    mut on_notice: impl FnMut(&ThemeNotice, &mut App) + 'static,
) -> Subscription {
    let emitter = notices(cx).emitter.clone();
    cx.subscribe(&emitter, move |_, notice: &ThemeNotice, cx| {
        on_notice(notice, cx);
    })
}

/// The unknown-theme notice raised before the caller subscribed and not resolved since.
pub fn pending_unknown_theme(cx: &mut App) -> Option<ThemeNotice> {
    notices(cx).pending.clone()
}

/// Records that the first scan of the user themes finished.
pub(crate) fn mark_scanned(cx: &mut App) {
    notices(cx).scanned = true;
}

/// Reports the outcome of resolving the selected theme name: `missing` is `Some((name,
/// fallback))` when the name is unknown.
pub(crate) fn report_resolution(cx: &mut App, missing: Option<(&str, &str)>) {
    let state = notices(cx);
    if !state.scanned {
        return;
    }
    let Some((name, fallback)) = missing else {
        state.reported.clear();
        state.pending = None;
        return;
    };
    state.reported.retain(|reported| reported == name);
    if !state.reported.insert(name.to_owned()) {
        return;
    }
    tracing::warn!(
        theme = name,
        fallback,
        "unknown theme name; using the default"
    );
    let notice = ThemeNotice::UnknownTheme {
        name: name.to_owned(),
        fallback: fallback.to_owned(),
    };
    state.pending = Some(notice.clone());
    let emitter = state.emitter.clone();
    emitter.update(cx, |_, cx| cx.emit(notice));
}
