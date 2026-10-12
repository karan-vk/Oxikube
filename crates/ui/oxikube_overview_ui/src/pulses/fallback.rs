//! The one-off count behind a tile whose kind has no feed.
//!
//! A kind the watch budget refused (or whose feed failed, or that nothing opened) cannot be
//! counted from a cache. Rather than start a feed for it, the page asks the API server once for
//! the count ([`fetch_count`]: a metadata list with `limit=1`) on the Tokio bridge, shows it
//! marked as counted once, and asks again every [`FALLBACK_INTERVAL`] or on `r`. The request is
//! owned by the page: closing it aborts the one in flight.

use oxikube_app::fetch_count;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_runtime::{notify_coalesced, spawn_kube};

use gpui::Context;

use super::FALLBACK_INTERVAL;
use super::face::Fallback;
use super::view::PulsesView;

impl PulsesView {
    /// Asks for the one-off count of each tile that wants one and has not had one lately.
    pub(super) fn fetch_due(&mut self, now: f64, cx: &mut Context<Self>) {
        let settled = self.settled();
        for index in self.model.wanting_fallback(settled) {
            let due =
                self.fetched_at[index].is_none_or(|at| now - at >= FALLBACK_INTERVAL.as_secs_f64());
            if due && !self.fetching[index] {
                self.fetch(index, now, cx);
            }
        }
    }

    /// `r`: counts again every kind that has no feed, now.
    pub fn recount(&mut self, cx: &mut Context<Self>) {
        let now = self.now();
        let settled = self.settled();
        for index in self.model.wanting_fallback(settled) {
            self.fetch(index, now, cx);
        }
    }

    fn fetch(&mut self, index: usize, now: f64, cx: &mut Context<Self>) {
        let Some(target) = self.model.tiles()[index].spec.target().cloned() else {
            return;
        };
        let Some(session) = self.deps.sessions.get(&self.cluster) else {
            return;
        };
        let (Some(reader), selection) =
            (session.resources(), session.namespace_selection().clone())
        else {
            return;
        };
        let id = self.model.tiles()[index].spec.id;
        self.fetched_at[index] = Some(now);
        // Keep what the tile shows while it counts again; show the wait only the first time.
        if self.model.tiles()[index].fallback == Fallback::None
            && self.model.set_fallback(id, Fallback::Loading)
        {
            notify_coalesced(cx);
        }
        let read = spawn_kube(cx, async move {
            fetch_count(&*reader, &target, &selection).await
        });
        self.fetching[index] = true;
        self.fetches[index] = Some(cx.spawn(async move |this, cx| {
            let result = match read.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |this, cx| this.fetched(index, result, cx))
                .ok();
        }));
    }

    fn fetched(
        &mut self,
        index: usize,
        result: OxiResult<oxikube_app::FetchedCount>,
        cx: &mut Context<Self>,
    ) {
        let id = self.model.tiles()[index].spec.id;
        let fallback = match result {
            Ok(count) => Fallback::Counted(count),
            Err(error) => {
                if error.kind() != ErrorKind::Forbidden {
                    tracing::debug!(%error, tile = id, "counting a kind once failed");
                }
                Fallback::Failed(error.message().to_owned())
            }
        };
        let counted = matches!(fallback, Fallback::Counted(_));
        if self.model.set_fallback(id, fallback) {
            if counted {
                let now = self.now();
                self.model.sample(now);
            }
            notify_coalesced(cx);
        }
        // The request is over: the next one may start. The task stays in its slot until that one
        // replaces it; clearing it from inside would cancel it mid-flight.
        self.fetching[index] = false;
    }
}
