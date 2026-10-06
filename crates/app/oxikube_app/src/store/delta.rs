//! What a [`Subscription`](super::Subscription) yields: [`StoreDelta`] (the story's "Delta"),
//! its [`RowChange`] of [`RowOp`]s with index positions, and the [`FeedState`].

use std::sync::Arc;

use oxikube_domain::{ErrorKind, OxiError};

use super::feed::TableColumns;
use super::object::StoreObject;

/// The health of the feed(s) behind a subscription, so a view can tell loading from empty from
/// forbidden (E07-S10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedState {
    /// Opening or (re)listing; rows may be stale or missing.
    Warming,
    /// Listed and watching; rows are current.
    Ready,
    /// A watch error the adapter or the store is retrying; rows are kept.
    Retrying {
        /// What failed (already redacted by the adapter).
        message: String,
    },
    /// The user may not list this kind here (`403` or rejected credentials). Not retried until
    /// the view subscribes again.
    Forbidden {
        /// The server's reason.
        message: String,
    },
    /// The credentials were rejected or have expired (`401`, an exec plugin that needs a login):
    /// an auth problem, not a missing permission, and the table says so (k9s #3730). Not retried
    /// until the view subscribes again (or [`Subscription::retry`](super::Subscription::retry)).
    Unauthorized {
        /// What the server or the credential plugin said (already redacted by the adapter).
        message: String,
    },
    /// A failure that is not retried until the view subscribes again (unknown kind, refused by
    /// the watch budget, ...).
    Failed {
        /// The error kind.
        kind: ErrorKind,
        /// What failed.
        message: String,
    },
}

impl FeedState {
    /// The state an error leaves a feed in; `retrying` says whether the feed carries on.
    pub(crate) fn from_error(error: &OxiError, retrying: bool) -> Self {
        let message = error.message().to_owned();
        match (retrying, error.kind()) {
            (true, _) => FeedState::Retrying { message },
            (false, ErrorKind::Forbidden) => FeedState::Forbidden { message },
            (false, ErrorKind::Auth) => FeedState::Unauthorized { message },
            (false, kind) => FeedState::Failed { kind, message },
        }
    }

    /// Whether the rows are current.
    pub fn is_ready(&self) -> bool {
        matches!(self, FeedState::Ready)
    }

    /// Whether the feed has stopped and will not recover by itself.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            FeedState::Forbidden { .. } | FeedState::Unauthorized { .. } | FeedState::Failed { .. }
        )
    }

    /// A short label for logs (never the message, which may quote the server).
    pub(crate) fn label(&self) -> &'static str {
        match self {
            FeedState::Warming => "warming",
            FeedState::Ready => "ready",
            FeedState::Retrying { .. } => "retrying",
            FeedState::Forbidden { .. } => "forbidden",
            FeedState::Unauthorized { .. } => "unauthorized",
            FeedState::Failed { .. } => "failed",
        }
    }

    /// How bad the state is, for combining the parts of a multi-namespace subscription: the
    /// subscription shows its worst part.
    pub(crate) fn severity(&self) -> u8 {
        match self {
            FeedState::Ready => 0,
            FeedState::Warming => 1,
            FeedState::Retrying { .. } => 2,
            FeedState::Failed { .. } => 3,
            FeedState::Forbidden { .. } => 4,
            FeedState::Unauthorized { .. } => 5,
        }
    }
}

/// One edit of a subscriber's sorted row list. Positions refer to the list as it is after the
/// previous op, so ops must be applied in order.
#[derive(Debug, Clone, PartialEq)]
pub enum RowOp {
    /// Insert `object` at `index`.
    Insert {
        /// Position after insertion.
        index: usize,
        /// The new row.
        object: Arc<StoreObject>,
    },
    /// Replace the row at `index` (its sort position did not change).
    Update {
        /// Position of the row.
        index: usize,
        /// The new version.
        object: Arc<StoreObject>,
    },
    /// Remove the row at `index`.
    Remove {
        /// Position of the removed row.
        index: usize,
    },
}

impl RowOp {
    /// Applies this op to `rows` (a consumer's copy of the list).
    ///
    /// # Panics
    ///
    /// If `index` is out of bounds, which means the ops were applied out of order or to a list
    /// that missed an earlier item.
    pub fn apply(&self, rows: &mut Vec<Arc<StoreObject>>) {
        match self {
            RowOp::Insert { index, object } => rows.insert(*index, object.clone()),
            RowOp::Update { index, object } => rows[*index] = object.clone(),
            RowOp::Remove { index } => {
                rows.remove(*index);
            }
        }
    }
}

/// How the rows changed since the previous item.
#[derive(Debug, Clone, PartialEq)]
pub enum RowChange {
    /// The whole list, sorted and filtered: replace what you have. The first item of every
    /// subscription, and any item after a relist, a filter/sort/scope change, or a burst too
    /// large to be worth expressing as ops.
    Snapshot(Vec<Arc<StoreObject>>),
    /// Ops to apply in order (one coalesced batch, however many feed events produced it).
    Ops(Vec<RowOp>),
    /// Rows unchanged; only the state or the columns moved.
    Unchanged,
}

/// One item of a subscription stream: a coalesced batch of row changes plus the feed state.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreDelta {
    /// The rows' change.
    pub rows: RowChange,
    /// The feed state now.
    pub state: FeedState,
    /// Table columns: `Some` on the first item that knows them and whenever they change; `None`
    /// means unchanged (and always `None` for reflector feeds).
    pub columns: Option<TableColumns>,
    /// The number of rows after this item is applied.
    pub len: usize,
    /// How many objects the feeds hold before the in-app filter (the "of" in `123 of 4,812`).
    /// Equals `len` without a filter. Maintained from the caches, never from rendering rows.
    pub total: usize,
}

impl StoreDelta {
    /// Applies the row change to `rows` (a consumer's copy of the list).
    pub fn apply_to(&self, rows: &mut Vec<Arc<StoreObject>>) {
        match &self.rows {
            RowChange::Snapshot(all) => rows.clone_from(all),
            RowChange::Ops(ops) => ops.iter().for_each(|op| op.apply(rows)),
            RowChange::Unchanged => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_states() {
        let forbidden = OxiError::forbidden("pods is forbidden");
        assert!(matches!(
            FeedState::from_error(&forbidden, false),
            FeedState::Forbidden { .. }
        ));
        let expired = OxiError::auth("token expired", false);
        assert!(matches!(
            FeedState::from_error(&expired, false),
            FeedState::Unauthorized { .. }
        ));
        let net = OxiError::network("reset");
        assert!(matches!(
            FeedState::from_error(&net, true),
            FeedState::Retrying { .. }
        ));
        let gone = OxiError::not_found("no such kind");
        let state = FeedState::from_error(&gone, false);
        assert!(state.is_terminal());
        assert!(matches!(
            state,
            FeedState::Failed {
                kind: ErrorKind::NotFound,
                ..
            }
        ));
        assert!(FeedState::Ready.is_ready());
        assert!(
            FeedState::Forbidden {
                message: String::new()
            }
            .severity()
                > FeedState::Warming.severity()
        );
    }
}
