//! [`follow_settings`]: `logs.buffer_lines` (global and per cluster) and `logs.max_streams` into the
//! running `LogService`, off the UI thread.

use std::sync::Arc;

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{App, AppContext as _};
use oxikube_app::logs::LogService;
use oxikube_domain::ids::ClusterId;
use oxikube_settings::{Settings as _, SettingsStore};

use crate::LogsSettings;

/// The bounds the settings say: the buffer's default and the clusters' own, and the stream cap.
struct Bounds {
    default: usize,
    max_streams: usize,
    clusters: Vec<(ClusterId, usize)>,
}

/// Sets the service's bounds from the current settings and again on every change of them:
/// sessions that are open are trimmed or given room at once, new ones start with the new bound.
/// The clusters that carry their own `clusters.<id>.logs.buffer_lines` get theirs; the rest follow
/// `logs.buffer_lines`. Without a settings store the service keeps the bound it was built with.
///
/// Shrinking a buffer drops its oldest lines, which for a large buffer is real work, so the
/// resize runs on a background task that applies the newest bounds in order; the settings
/// observer only reads them (a few hash lookups) and sends. The task ends with the observer.
pub fn follow_settings(service: &Arc<LogService>, cx: &mut App) {
    let (tx, mut rx) = unbounded::<Bounds>();
    let service = service.clone();
    cx.background_spawn(async move {
        while let Some(mut bounds) = rx.next().await {
            // Only the newest matters when several changes queued up.
            while let Ok(newer) = rx.try_recv() {
                bounds = newer;
            }
            service.set_buffer_lines(bounds.default);
            service.set_max_streams(bounds.max_streams);
            service.set_cluster_buffer_lines(bounds.clusters);
        }
    })
    .detach();
    send(&tx, cx);
    LogsSettings::observe(cx, move |cx| send(&tx, cx)).detach();
}

fn send(tx: &UnboundedSender<Bounds>, cx: &App) {
    let Some(store) = cx.try_global::<SettingsStore>() else {
        return;
    };
    let Some(global) = store.try_get::<LogsSettings>(None) else {
        return;
    };
    let clusters = store
        .cluster_values::<LogsSettings>()
        .filter_map(|(id, settings)| Some((id.parse::<ClusterId>().ok()?, settings.buffer_lines)))
        .collect();
    // The task is gone only when the app is shutting down.
    let _ = tx.unbounded_send(Bounds {
        default: global.buffer_lines,
        max_streams: global.max_streams,
        clusters,
    });
}
