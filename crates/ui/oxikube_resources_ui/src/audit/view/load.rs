//! Reading the log: the first page, older pages as the table scrolls, the newest page after a
//! signal, and what each does to the rows. Every read runs on the Tokio bridge (`spawn_kube`),
//! never on the UI thread, and a read that lands after the filters changed is dropped.

use std::collections::VecDeque;

use futures::future::Either;
use gpui::{App, AppContext as _, Context};
use oxikube_app::audit::query::{NewRecords, PAGE_SIZE, READ_TIMEOUT, count_new};
use oxikube_domain::OxiResult;
use oxikube_domain::audit::AuditRecord;
use oxikube_ports::AuditPage;
use oxikube_runtime::{KubeTaskError, NotifyCoalescedExt as _, spawn_kube};

use super::{AuditView, Phase};

/// What a read comes back as, flattened to a message for the screen.
fn flatten(result: Result<OxiResult<AuditPage>, KubeTaskError>) -> Result<AuditPage, String> {
    match result {
        Ok(Ok(page)) => Ok(page),
        Ok(Err(error)) => Err(error.message().to_owned()),
        Err(error) => Err(error.to_string()),
    }
}

impl AuditView {
    /// Reads the newest page again for the current filters and replaces the rows with it.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.more_task = None;
        self.table.update_quiet(cx, |d| d.loading_more = false);
        if self.model(cx).is_empty() {
            self.phase = Phase::Loading;
        }
        let (reader, filters, now) = (
            self.deps.reader.clone(),
            self.filters.clone(),
            self.deps.clock.now(),
        );
        let work = spawn_kube(cx, async move {
            reader.page(&filters, now, None, PAGE_SIZE).await
        });
        self.first_task = Some(cx.spawn(async move |this, cx| {
            let result = bounded(work, cx.background_executor().timer(READ_TIMEOUT)).await;
            this.update(cx, |view, cx| view.landed_first(generation, result, cx))
                .ok();
        }));
        cx.notify();
    }

    fn landed_first(
        &mut self,
        generation: u64,
        page: Result<AuditPage, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        self.phase = Phase::Ready;
        match page {
            Ok(page) => {
                self.error = None;
                self.pending_new = None;
                let names = self.cluster_names();
                let mut old = VecDeque::new();
                self.table.update_quiet(cx, |d| {
                    d.names = names;
                    old = d.model.replace(page.records, page.next);
                });
                release_off_thread(old, cx);
                self.sync_batch(cx);
            }
            Err(message) => {
                tracing::warn!(%message, "reading the audit log failed");
                self.error = Some(message.into());
            }
        }
        cx.notify();
    }

    /// Reads the page after the loaded ones (the table asked: the visible rows near the end).
    pub(crate) fn load_more(&mut self, cx: &mut Context<Self>) {
        let model = self.model(cx);
        let Some(cursor) = model.cursor().filter(|_| model.has_more()) else {
            return;
        };
        if self.table.delegate(cx).loading_more {
            return;
        }
        self.table.update_quiet(cx, |d| d.loading_more = true);
        let generation = self.generation;
        let (reader, filters, now) = (
            self.deps.reader.clone(),
            self.filters.clone(),
            self.deps.clock.now(),
        );
        let work = spawn_kube(cx, async move {
            reader.page(&filters, now, Some(cursor), PAGE_SIZE).await
        });
        self.more_task = Some(cx.spawn(async move |this, cx| {
            let result = bounded(work, cx.background_executor().timer(READ_TIMEOUT)).await;
            this.update(cx, |view, cx| view.landed_more(generation, result, cx))
                .ok();
        }));
    }

    fn landed_more(
        &mut self,
        generation: u64,
        page: Result<AuditPage, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        match page {
            Ok(page) => {
                self.error = None;
                self.table.update_quiet(cx, |d| {
                    d.loading_more = false;
                    d.model.append(page.records, page.next);
                });
            }
            Err(message) => {
                tracing::warn!(%message, "reading older audit records failed");
                self.table.update_quiet(cx, |d| d.loading_more = false);
                self.error = Some(message.into());
            }
        }
        cx.notify_coalesced();
    }

    /// Listens for [`AuditAppended`](oxikube_app::AuditAppended) for as long as the view lives.
    /// The signal coalesces, so a burst of flushes wakes this once.
    pub(super) fn start_live(&mut self, cx: &mut Context<Self>) {
        let mut listener = self.deps.listener.clone();
        self.live_task = Some(cx.spawn(async move |this, cx| {
            while listener.changed().await.is_some() {
                if this.update(cx, |view, cx| view.refresh_head(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// Reads the newest page after records were stored. One read at a time: a signal that comes
    /// while one is in flight runs one more read when it lands.
    pub(crate) fn refresh_head(&mut self, cx: &mut Context<Self>) {
        if self.error.is_some() {
            // The log could not be read before: start over, it may have recovered.
            self.reload(cx);
            return;
        }
        if self.head_busy {
            self.head_dirty = true;
            return;
        }
        self.head_busy = true;
        let generation = self.generation;
        let (reader, filters, now) = (
            self.deps.reader.clone(),
            self.filters.clone(),
            self.deps.clock.now(),
        );
        let work = spawn_kube(cx, async move {
            reader.page(&filters, now, None, PAGE_SIZE).await
        });
        self.head_task = Some(cx.spawn(async move |this, cx| {
            let result = bounded(work, cx.background_executor().timer(READ_TIMEOUT)).await;
            this.update(cx, |view, cx| view.landed_head(generation, result, cx))
                .ok();
        }));
    }

    fn landed_head(
        &mut self,
        generation: u64,
        page: Result<AuditPage, String>,
        cx: &mut Context<Self>,
    ) {
        self.head_busy = false;
        if generation == self.generation {
            match page {
                Ok(page) => self.apply_head(page, cx),
                Err(message) => tracing::debug!(%message, "refreshing the audit viewer failed"),
            }
        }
        if std::mem::take(&mut self.head_dirty) {
            self.refresh_head(cx);
        }
    }

    /// Puts what is new in `fresh` (the newest page) in front when the viewer is scrolled to the
    /// top; otherwise counts it for the pill.
    fn apply_head(&mut self, fresh: AuditPage, cx: &mut Context<Self>) {
        let at_top = self.first_visible == 0;
        let news = count_new(self.model(cx).head(), &fresh.records);
        if news.is_none() {
            return;
        }
        let empty = self.model(cx).is_empty();
        match news {
            _ if !at_top && !empty => self.pending_new = Some(news),
            NewRecords::Exactly(n) if !empty => {
                let mut records = fresh.records;
                records.truncate(n);
                self.table.update_quiet(cx, |d| d.model.prepend(records));
                self.pending_new = None;
            }
            // Nothing loaded yet, or the whole newest page is new: it is the new first page.
            NewRecords::Exactly(_) | NewRecords::AtLeast(_) => {
                let mut old = VecDeque::new();
                self.table
                    .update_quiet(cx, |d| old = d.model.replace(fresh.records, fresh.next));
                release_off_thread(old, cx);
                self.pending_new = None;
            }
        }
        cx.notify_coalesced();
    }

    /// The "N new" pill: scrolls to the top and reads the newest page.
    pub(crate) fn show_new(&mut self, cx: &mut Context<Self>) {
        self.pending_new = None;
        self.first_visible = 0;
        if !self.model(cx).is_empty() {
            self.table.scroll_to_row(0, cx);
        }
        self.reload(cx);
    }

    /// The table scrolled: the first visible row is `first`.
    pub(crate) fn on_scrolled(&mut self, first: usize, cx: &mut Context<Self>) {
        self.first_visible = first;
        if first == 0 && self.pending_new.is_some() {
            self.show_new(cx);
        }
    }
}

/// Frees `records` on a background thread: a replaced page of tens of thousands of records owns
/// hundreds of thousands of strings, too much to free inside one frame.
fn release_off_thread(records: VecDeque<AuditRecord>, cx: &App) {
    if records.len() > RELEASE_INLINE_MAX {
        cx.background_spawn(async move { drop(records) }).detach();
    }
}

/// Up to this many records are cheap enough to free in place.
const RELEASE_INLINE_MAX: usize = 256;

/// Awaits `work` unless `timer` fires first, which is an error message for the screen: a store
/// that never answers must not leave "Loading…" up for ever. Dropping `work` aborts the read.
pub(in crate::audit) async fn bounded(
    work: impl Future<Output = Result<OxiResult<AuditPage>, KubeTaskError>>,
    timer: impl Future<Output = ()>,
) -> Result<AuditPage, String> {
    futures::pin_mut!(work, timer);
    match futures::future::select(work, timer).await {
        Either::Left((result, _)) => flatten(result),
        Either::Right(((), _)) => Err("reading the audit log timed out".to_owned()),
    }
}
