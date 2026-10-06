//! [`TableState`]: what the table is in, derived from the feed's state, the row count and the
//! active filter. Pure, so every cell of the state matrix is a plain unit test.

use oxikube_app::store::FeedState;
use oxikube_domain::ErrorKind;

/// Why rows that are still shown may be out of date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stale {
    /// The feed is (re)listing; the rows are from before.
    Refreshing,
    /// The watch dropped and is being reopened; the rows are from before.
    Reconnecting {
        /// What failed (redacted by the adapter).
        message: String,
    },
    /// The user may no longer list the kind; these are the last rows seen.
    Forbidden,
    /// The credentials stopped working; these are the last rows seen.
    Unauthorized,
    /// The feed stopped with an error; these are the last rows seen.
    Failed {
        /// What failed.
        message: String,
    },
}

/// What a resource table shows. Four distinct "no rows" states, so a blank table is never
/// ambiguous (k9s #4121), plus the rows themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableState {
    /// The feed is listing for the first time: skeleton rows and a spinner.
    Loading,
    /// The first watch failed and is being retried automatically.
    Reconnecting {
        /// What failed (redacted by the adapter).
        message: String,
    },
    /// Listed, and there are none (in this scope).
    Empty,
    /// Listed, and the active filter hides every object.
    FilteredEmpty {
        /// The filter, as the user would read it.
        filter: String,
    },
    /// `403`: the user may not list this kind here.
    Forbidden {
        /// The server's reason.
        message: String,
    },
    /// `401` or an expired credential: an auth problem, not a missing permission.
    Unauthorized {
        /// What the server or the credential plugin said.
        message: String,
    },
    /// The feed failed for another reason (transport, timeout, unknown kind, watch budget).
    Failed {
        /// The error kind, for the copy.
        kind: ErrorKind,
        /// What failed.
        message: String,
    },
    /// There are rows to show; `stale` says when they may not be current.
    Rows {
        /// Why the rows may be out of date, if they may be.
        stale: Option<Stale>,
    },
}

impl TableState {
    /// The state for a feed in `feed` with `rows` rows after the filter, where `filter` is the
    /// active filter's text (`None` or empty for none).
    ///
    /// Rows win over errors: while there are rows they stay on screen and the state only marks
    /// them stale. Without rows, the feed state decides, and a ready, empty feed is
    /// [`FilteredEmpty`](Self::FilteredEmpty) when a filter is active (it may hide everything)
    /// and [`Empty`](Self::Empty) otherwise.
    pub fn derive(feed: &FeedState, rows: usize, filter: Option<&str>) -> Self {
        let filter = filter.filter(|f| !f.is_empty());
        if rows > 0 {
            return TableState::Rows {
                stale: match feed {
                    FeedState::Ready => None,
                    FeedState::Warming => Some(Stale::Refreshing),
                    FeedState::Retrying { message } => Some(Stale::Reconnecting {
                        message: message.clone(),
                    }),
                    FeedState::Forbidden { .. } => Some(Stale::Forbidden),
                    FeedState::Unauthorized { .. } => Some(Stale::Unauthorized),
                    FeedState::Failed {
                        kind: ErrorKind::Forbidden,
                        ..
                    } => Some(Stale::Forbidden),
                    FeedState::Failed {
                        kind: ErrorKind::Auth,
                        ..
                    } => Some(Stale::Unauthorized),
                    FeedState::Failed { message, .. } => Some(Stale::Failed {
                        message: message.clone(),
                    }),
                },
            };
        }
        match feed {
            FeedState::Warming => TableState::Loading,
            FeedState::Retrying { message } => TableState::Reconnecting {
                message: message.clone(),
            },
            FeedState::Ready => match filter {
                Some(filter) => TableState::FilteredEmpty {
                    filter: filter.to_owned(),
                },
                None => TableState::Empty,
            },
            FeedState::Forbidden { message } => TableState::Forbidden {
                message: message.clone(),
            },
            FeedState::Unauthorized { message } => TableState::Unauthorized {
                message: message.clone(),
            },
            // Mapped from the error taxonomy, never by matching the message (k9s #3730).
            FeedState::Failed {
                kind: ErrorKind::Forbidden,
                message,
            } => TableState::Forbidden {
                message: message.clone(),
            },
            FeedState::Failed {
                kind: ErrorKind::Auth,
                message,
            } => TableState::Unauthorized {
                message: message.clone(),
            },
            FeedState::Failed { kind, message } => TableState::Failed {
                kind: *kind,
                message: message.clone(),
            },
        }
    }

    /// Whether a "Retry" makes sense: the feed has stopped or is failing, not merely loading or
    /// empty.
    pub fn can_retry(&self) -> bool {
        match self {
            TableState::Reconnecting { .. }
            | TableState::Forbidden { .. }
            | TableState::Unauthorized { .. }
            | TableState::Failed { .. } => true,
            TableState::Rows { stale } => matches!(
                stale,
                Some(
                    Stale::Reconnecting { .. }
                        | Stale::Forbidden
                        | Stale::Unauthorized
                        | Stale::Failed { .. }
                )
            ),
            TableState::Loading | TableState::Empty | TableState::FilteredEmpty { .. } => false,
        }
    }

    /// The staleness of the rows being shown, if any.
    pub fn stale(&self) -> Option<&Stale> {
        match self {
            TableState::Rows { stale } => stale.as_ref(),
            _ => None,
        }
    }

    /// Whether the state wants the animated spinner (loading, reconnecting, refreshing).
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            TableState::Loading
                | TableState::Reconnecting { .. }
                | TableState::Rows {
                    stale: Some(Stale::Refreshing | Stale::Reconnecting { .. })
                }
        )
    }
}
