//! The state value of a filter. (That the jump bar and the filter bar agree is tested with the
//! jump bar: `jump/tests/filter.rs`.)

use super::{FilterError, FilterState};

#[test]
fn an_error_keeps_the_last_good_filter() {
    let mut state = FilterState::new();
    assert!(!state.is_active());
    assert!(state.edit("web"));
    assert!(state.is_active());
    let good = state.parts().clone();

    assert!(!state.edit("web("));
    assert!(matches!(state.error(), Some(FilterError::Regex(_))));
    assert_eq!(
        state.parts(),
        &good,
        "the rows of the last good filter stay"
    );
    assert_eq!(state.text(), "web(");

    assert!(state.edit("web-1"));
    assert!(state.error().is_none());
    assert_ne!(state.parts(), &good);
}

#[test]
fn a_bare_slash_and_clear_remove_the_filter() {
    let mut state = FilterState::new();
    state.edit("-l app=x");
    assert!(state.is_active());
    assert!(state.edit("/"), "a bare slash is no filter, not an error");
    assert!(!state.is_active());

    state.edit("web");
    state.clear();
    assert_eq!(state, FilterState::new());
}

#[test]
fn a_bare_bang_is_an_error_that_keeps_the_filter() {
    let mut state = FilterState::new();
    state.edit("web");
    assert!(!state.edit("!"));
    assert_eq!(state.error(), Some(&FilterError::NothingToInvert));
    assert!(state.is_active());
}
