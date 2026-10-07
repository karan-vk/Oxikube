//! Building the match index of a pattern over the lines a session already holds, and putting it
//! in the view.
//!
//! Small buffers are tested on the spot (a few thousand regex tests are well under a frame);
//! larger ones in [`CHUNK`]-line jobs on the background executor, each holding the session's lock
//! only while it tests its chunk. The finished index is published in one update, caught up to the
//! lines that arrived meanwhile. Until then the previous index keeps serving.

use gpui::{AppContext as _, Context};
use oxikube_app::logs::MatchIndex;
use oxikube_runtime::notify_coalesced;

use crate::LogView;

/// Lines tested on the UI thread without a background job: about 0.2 ms of regex at typical
/// line lengths.
pub const INLINE_LINES: usize = 4_000;

/// Lines one background job tests before it lets go of the session's lock.
pub const CHUNK: usize = 16_384;

impl LogView {
    /// The pattern changed (or the session did): builds the index for it and shows its effect.
    pub(crate) fn rematch(&mut self, cx: &mut Context<Self>) {
        self.search.scan = None;
        self.search.scanning = false;
        let Some(matcher) = self.search.state.matcher().cloned() else {
            self.install_index(None, cx);
            return;
        };
        let Some(reader) = self.session.as_ref().map(|session| session.reader()) else {
            self.install_index(Some(MatchIndex::new(matcher)), cx);
            return;
        };
        let mut index = MatchIndex::new(matcher);
        let pending = reader.read(|buffer, _| index.pending(buffer));
        if pending <= INLINE_LINES as u64 {
            reader.read(|buffer, _| index.catch_up(buffer));
            self.install_index(Some(index), cx);
            return;
        }
        self.search.scanning = true;
        let id = reader.id();
        self.search.scan = Some(cx.spawn(async move |this, cx| {
            let index = loop {
                let job = reader.clone();
                let (done, rest) = cx
                    .background_spawn(async move {
                        let rest = job.read(|buffer, _| {
                            index.scan(buffer, CHUNK);
                            index.pending(buffer)
                        });
                        (index, rest)
                    })
                    .await;
                index = done;
                if rest <= INLINE_LINES as u64 {
                    break index;
                }
            };
            this.update(cx, |view, cx| view.finish_scan(id, index, cx))
                .ok();
        }));
        notify_coalesced(cx);
    }

    /// Publishes an index built in the background, unless the session it read has gone.
    fn finish_scan(&mut self, session: u64, mut index: MatchIndex, cx: &mut Context<Self>) {
        let Some(reader) = self.session.as_ref().map(|session| session.reader()) else {
            return;
        };
        if reader.id() != session {
            return;
        }
        reader.read(|buffer, _| index.catch_up(buffer));
        self.install_index(Some(index), cx);
    }

    /// Carries `index` in the window (narrowing the rows in filter mode) and rebuilds what the
    /// renderers keep per row, keeping the line at the top of the screen where it is.
    pub(crate) fn install_index(&mut self, index: Option<MatchIndex>, cx: &mut Context<Self>) {
        self.search.scanning = false;
        let anchor = self.top_seq();
        self.window
            .set_index(index, self.search.state.is_filtering());
        self.relevel();
        self.rows_rebuilt(anchor);
        notify_coalesced(cx);
    }

    /// The mode changed: the same index, rows narrowed or not.
    pub(crate) fn remode(&mut self, cx: &mut Context<Self>) {
        if self.search.scanning {
            // The finished index reads the mode when it is installed.
            notify_coalesced(cx);
            return;
        }
        let anchor = self.top_seq();
        self.window.set_narrowed(self.search.state.is_filtering());
        self.relevel();
        self.rows_rebuilt(anchor);
        notify_coalesced(cx);
    }

    /// A new session opened: its seqs start over, so does the index (empty; the deltas fill it).
    pub(crate) fn reindex_empty(&mut self) {
        self.search.scan = None;
        self.search.scanning = false;
        let matcher = self.search.state.matcher().cloned();
        self.window.set_index(
            matcher.map(MatchIndex::new),
            self.search.state.is_filtering(),
        );
        self.search.state.set_current(None);
    }
}
