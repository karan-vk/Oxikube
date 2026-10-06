//! Tests of the filter grammar, the name predicates and the fuzzy ranking: pure, no store.

mod fuzzy;
mod grammar;
mod narrowing;

use super::{FilterExpr, FilterParts, parse};

/// `input` parsed, which must succeed.
pub(super) fn ok(input: &str) -> FilterExpr {
    parse(input).unwrap_or_else(|e| panic!("{input:?} should parse: {e}"))
}

/// The parts of `input`.
pub(super) fn parts(input: &str) -> FilterParts {
    ok(input).parts()
}

/// Whether `input` lets a name through (client-side part only).
pub(super) fn passes(input: &str, name: &str) -> bool {
    let parts = parts(input);
    parts
        .filter
        .pattern
        .as_ref()
        .is_none_or(|p| p.matches(name))
}
