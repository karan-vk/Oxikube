//! Following the store's `Warning:` headers.
//!
//! The store hands every table of a session the same deduplicated stream
//! ([`ResourceStore::warnings`](oxikube_app::store::ResourceStore::warnings)): the first
//! occurrence of each distinct text reaches exactly one table, so one toast per message per
//! session. The table forwards it as [`ResourceTableEvent::ApiWarning`] and the window's
//! [`ResourceViews`](crate::ResourceViews) shows it. The stream is subscribed before the table's
//! feed opens, because nothing is replayed.

use futures::StreamExt as _;
use futures::stream::BoxStream;
use gpui::{AsyncApp, Context, Task, WeakEntity};
use oxikube_ports::ApiWarning;

use crate::table::view::{ResourceTable, ResourceTableEvent};

/// The task that forwards `warnings` as events until the table goes or the stream ends.
pub(in crate::table) fn poll_warnings(
    mut warnings: BoxStream<'static, ApiWarning>,
    cx: &mut Context<ResourceTable>,
) -> Task<()> {
    cx.spawn(
        async move |this: WeakEntity<ResourceTable>, cx: &mut AsyncApp| {
            while let Some(warning) = warnings.next().await {
                let alive =
                    this.update(cx, |_, cx| cx.emit(ResourceTableEvent::ApiWarning(warning)));
                if alive.is_err() {
                    break;
                }
            }
        },
    )
}
