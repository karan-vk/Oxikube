//! The state matrix: every feed state against zero and some rows, with and without a filter.

use oxikube_app::store::FeedState;
use oxikube_domain::{ErrorKind, OxiError};

use crate::table::states::{Stale, TableState};

fn msg(s: &str) -> String {
    s.to_owned()
}

#[test]
fn warming_with_no_rows_is_loading_not_empty() {
    assert_eq!(
        TableState::derive(&FeedState::Warming, 0, None),
        TableState::Loading
    );
}

#[test]
fn ready_with_no_rows_is_empty() {
    assert_eq!(
        TableState::derive(&FeedState::Ready, 0, None),
        TableState::Empty
    );
    assert_eq!(
        TableState::derive(&FeedState::Ready, 0, Some("")),
        TableState::Empty,
        "an empty filter is no filter"
    );
}

#[test]
fn ready_with_no_rows_under_a_filter_is_filtered_empty() {
    assert_eq!(
        TableState::derive(&FeedState::Ready, 0, Some("web")),
        TableState::FilteredEmpty { filter: msg("web") }
    );
}

#[test]
fn forbidden_unauthorized_and_failed_are_three_different_states() {
    let forbidden = FeedState::Forbidden {
        message: msg("pods is forbidden: User \"me\" cannot list resource \"pods\""),
    };
    let unauthorized = FeedState::Unauthorized {
        message: msg("the server has asked for the client to provide credentials"),
    };
    let failed = FeedState::Failed {
        kind: ErrorKind::Network,
        message: msg("connection refused"),
    };
    assert!(matches!(
        TableState::derive(&forbidden, 0, None),
        TableState::Forbidden { .. }
    ));
    assert!(matches!(
        TableState::derive(&unauthorized, 0, None),
        TableState::Unauthorized { .. }
    ));
    assert!(matches!(
        TableState::derive(&failed, 0, None),
        TableState::Failed {
            kind: ErrorKind::Network,
            ..
        }
    ));
}

#[test]
fn an_auth_error_reads_as_auth_whatever_its_message_says() {
    // k9s #3730: an expired token surfaced as "unknown resource". The state comes from the
    // error kind, not from the text.
    let error = OxiError::auth("the server could not find the requested resource", false);
    let feed = oxikube_app::store::FeedState::Unauthorized {
        message: error.message().to_owned(),
    };
    assert!(matches!(
        TableState::derive(&feed, 0, None),
        TableState::Unauthorized { .. }
    ));
    // A `Failed` feed that carries the auth kind still maps to auth, and a 403 kind to forbidden.
    let failed_auth = FeedState::Failed {
        kind: ErrorKind::Auth,
        message: msg("x"),
    };
    let failed_forbidden = FeedState::Failed {
        kind: ErrorKind::Forbidden,
        message: msg("forbidden"),
    };
    assert!(matches!(
        TableState::derive(&failed_auth, 0, None),
        TableState::Unauthorized { .. }
    ));
    assert!(matches!(
        TableState::derive(&failed_forbidden, 0, None),
        TableState::Forbidden { .. }
    ));
    // And a not-found failure whose message mentions credentials stays a plain failure.
    let not_found = FeedState::Failed {
        kind: ErrorKind::NotFound,
        message: msg("Unauthorized: the credentials were rejected"),
    };
    assert!(matches!(
        TableState::derive(&not_found, 0, None),
        TableState::Failed {
            kind: ErrorKind::NotFound,
            ..
        }
    ));
}

#[test]
fn a_failing_first_watch_is_reconnecting() {
    let retrying = FeedState::Retrying {
        message: msg("connection reset"),
    };
    assert_eq!(
        TableState::derive(&retrying, 0, None),
        TableState::Reconnecting {
            message: msg("connection reset")
        }
    );
}

#[test]
fn rows_stay_and_are_marked_stale_instead_of_cleared() {
    let ready = TableState::derive(&FeedState::Ready, 3, Some("web"));
    assert_eq!(ready, TableState::Rows { stale: None });
    let cases = [
        (FeedState::Warming, Stale::Refreshing),
        (
            FeedState::Retrying {
                message: msg("reset"),
            },
            Stale::Reconnecting {
                message: msg("reset"),
            },
        ),
        (
            FeedState::Forbidden { message: msg("no") },
            Stale::Forbidden,
        ),
        (
            FeedState::Unauthorized {
                message: msg("expired"),
            },
            Stale::Unauthorized,
        ),
        (
            FeedState::Failed {
                kind: ErrorKind::Timeout,
                message: msg("slow"),
            },
            Stale::Failed {
                message: msg("slow"),
            },
        ),
    ];
    for (feed, stale) in cases {
        assert_eq!(
            TableState::derive(&feed, 5, None),
            TableState::Rows { stale: Some(stale) },
            "{feed:?}"
        );
    }
}

#[test]
fn recovery_goes_back_to_plain_rows() {
    let failed = FeedState::Failed {
        kind: ErrorKind::Network,
        message: msg("down"),
    };
    assert!(TableState::derive(&failed, 0, None).can_retry());
    assert_eq!(
        TableState::derive(&FeedState::Ready, 4, None),
        TableState::Rows { stale: None }
    );
}

#[test]
fn only_failing_states_offer_retry_and_only_waiting_ones_spin() {
    let yes = [
        TableState::Reconnecting { message: msg("x") },
        TableState::Forbidden { message: msg("x") },
        TableState::Unauthorized { message: msg("x") },
        TableState::Failed {
            kind: ErrorKind::Internal,
            message: msg("x"),
        },
        TableState::Rows {
            stale: Some(Stale::Forbidden),
        },
        TableState::Rows {
            stale: Some(Stale::Reconnecting { message: msg("x") }),
        },
    ];
    for state in yes {
        assert!(state.can_retry(), "{state:?}");
    }
    let no = [
        TableState::Loading,
        TableState::Empty,
        TableState::FilteredEmpty { filter: msg("x") },
        TableState::Rows { stale: None },
        TableState::Rows {
            stale: Some(Stale::Refreshing),
        },
    ];
    for state in no {
        assert!(!state.can_retry(), "{state:?}");
    }
    assert!(TableState::Loading.is_busy());
    assert!(TableState::Reconnecting { message: msg("x") }.is_busy());
    assert!(
        TableState::Rows {
            stale: Some(Stale::Refreshing)
        }
        .is_busy()
    );
    assert!(!TableState::Empty.is_busy());
    assert!(!TableState::Forbidden { message: msg("x") }.is_busy());
}
