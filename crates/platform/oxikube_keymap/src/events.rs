//! The event the keymap raises when the problems with the user's `keymap.json` change.
//!
//! The keymap is platform code and cannot show a toast (layer rule), so it announces and the
//! binary shows: [`subscribe_diagnostics`] hands the binary a [`KeymapDiagnosticsEvent`] after
//! every load that changed the list of problems, including the load that fixed them (an empty
//! list, which clears the notification). A load that finds the same problems as the last one
//! raises nothing, so saving an unrelated edit does not pop the same toast again.
//!
//! Problems found before anyone subscribed (the start-up load) are not replayed; read them with
//! [`crate::user_diagnostics`] right after subscribing.

use gpui::{App, AppContext as _, Entity, EventEmitter, Global, Subscription};

use crate::diagnostics::{KeymapDiagnostic, KeymapDiagnosticsEvent};

/// The entity the event is emitted from.
struct Emitter;

impl EventEmitter<KeymapDiagnosticsEvent> for Emitter {}
impl EventEmitter<KeymapChanged> for Emitter {}

/// Raised after the keymap's bindings were replaced: a reload of `keymap.json` that changed
/// something, the vim layer switching, or an explicit [`crate::rebind`]. The keymap editor
/// re-reads its rows on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeymapChanged;

struct Events {
    emitter: Entity<Emitter>,
    /// The list the last event carried (empty before the first).
    last: Vec<KeymapDiagnostic>,
}

impl Global for Events {}

fn events(cx: &mut App) -> &mut Events {
    if !cx.has_global::<Events>() {
        let emitter = cx.new(|_| Emitter);
        cx.set_global(Events {
            emitter,
            last: Vec::new(),
        });
    }
    cx.global_mut::<Events>()
}

/// Run `on_event` whenever the problems with the user's `keymap.json` change. Keep the returned
/// subscription for as long as the handler should run.
pub fn subscribe_diagnostics(
    cx: &mut App,
    mut on_event: impl FnMut(&KeymapDiagnosticsEvent, &mut App) + 'static,
) -> Subscription {
    let emitter = events(cx).emitter.clone();
    cx.subscribe(&emitter, move |_, event: &KeymapDiagnosticsEvent, cx| {
        on_event(event, cx);
    })
}

/// Raise the event when `current` differs from what the last event carried.
pub(crate) fn publish(cx: &mut App, current: Vec<KeymapDiagnostic>) {
    let events = events(cx);
    if events.last == current {
        return;
    }
    events.last.clone_from(&current);
    let emitter = events.emitter.clone();
    emitter.update(cx, |_, cx| {
        cx.emit(KeymapDiagnosticsEvent {
            diagnostics: current,
        });
    });
}

/// Run `on_change` after every replacement of the keymap's bindings. Keep the returned
/// subscription for as long as the handler should run. Bindings installed before anyone
/// subscribed raise nothing: read the current rows after subscribing.
pub fn subscribe_changes(
    cx: &mut App,
    mut on_change: impl FnMut(&mut App) + 'static,
) -> Subscription {
    let emitter = events(cx).emitter.clone();
    cx.subscribe(&emitter, move |_, _: &KeymapChanged, cx| on_change(cx))
}

/// Raise [`KeymapChanged`].
pub(crate) fn publish_change(cx: &mut App) {
    let emitter = events(cx).emitter.clone();
    emitter.update(cx, |_, cx| cx.emit(KeymapChanged));
}
