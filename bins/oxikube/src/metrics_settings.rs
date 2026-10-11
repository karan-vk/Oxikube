//! `metrics.gpu_vendors` into the utilisation aggregator (E13-S02).
//!
//! `oxikube_settings` resolves the setting (globally and under `clusters.<id>.metrics`) and
//! `oxikube_app` has no `gpui`, so this binary carries the values across:
//! [`follow_gpu_vendors`] pushes the resolved
//! [`GpuVendorsTable`](oxikube_domain::utilisation::GpuVendorsTable) into the
//! [`GpuVendorsHandle`] now and after every change of the setting. The next computation of any
//! pod, node or namespace figure uses the new list; nothing reconnects.

use gpui::{App, Subscription};
use oxikube_app::GpuVendorsHandle;
use oxikube_settings::{MetricsSettings, Settings as _};

/// Pushes the GPU vendor lists into `vendors` now and whenever they change. Keep the returned
/// subscription, or `.detach()` it to follow for the life of the app. The push is a table swap
/// under a short lock, safe on the UI thread.
pub fn follow_gpu_vendors(cx: &mut App, vendors: GpuVendorsHandle) -> Subscription {
    vendors.set(MetricsSettings::gpu_table(cx));
    MetricsSettings::observe(cx, move |cx| {
        vendors.set(MetricsSettings::gpu_table(cx));
    })
}

#[cfg(test)]
mod tests;
