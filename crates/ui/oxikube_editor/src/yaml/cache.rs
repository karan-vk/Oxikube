//! `ParseCache`: memoises the last parse of a buffer by its version.

use std::sync::Arc;

use super::parse::parse_shared;
use super::result::ParseResult;

/// Keeps the [`ParseResult`] of the latest buffer version, so the validator, hover and
/// completion share one parse per edit instead of reparsing on every request.
#[derive(Debug, Default)]
pub struct ParseCache {
    last: Option<(u64, Arc<ParseResult>)>,
}

impl ParseCache {
    /// An empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The parse of `version`, reusing the cached one when the version matches; `text` is only
    /// called (and the buffer only parsed) on a miss.
    pub fn get_or_parse(
        &mut self,
        version: u64,
        text: impl FnOnce() -> Arc<str>,
    ) -> Arc<ParseResult> {
        if let Some((cached, result)) = &self.last
            && *cached == version
        {
            return Arc::clone(result);
        }
        let result = Arc::new(parse_shared(text()));
        self.last = Some((version, Arc::clone(&result)));
        result
    }

    /// The cached parse, if it is of `version`.
    #[must_use]
    pub fn get(&self, version: u64) -> Option<Arc<ParseResult>> {
        let (cached, result) = self.last.as_ref()?;
        (*cached == version).then(|| Arc::clone(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reparses_only_on_a_new_version() {
        let mut cache = ParseCache::new();
        let mut calls = 0;
        let first = cache.get_or_parse(1, || {
            calls += 1;
            Arc::from("a: 1")
        });
        let again = cache.get_or_parse(1, || {
            calls += 1;
            Arc::from("ignored")
        });
        assert!(Arc::ptr_eq(&first, &again));
        assert_eq!(calls, 1);
        let next = cache.get_or_parse(2, || Arc::from("b: 2"));
        assert_eq!(next.text(), "b: 2");
        assert!(cache.get(1).is_none());
        assert!(cache.get(2).is_some());
    }
}
