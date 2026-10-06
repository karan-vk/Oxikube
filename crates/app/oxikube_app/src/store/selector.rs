//! [`LabelSelector`]: kubectl's `-l` grammar, evaluated in the app against cached labels.
//!
//! Supports equality (`k=v`, `k==v`, `k!=v`), existence (`k`, `!k`) and set terms
//! (`k in (a,b)`, `k notin (a,b)`), comma-separated and AND-ed, as in
//! `apimachinery/pkg/labels`. The store uses the equality terms to pick a candidate set from its
//! label index before evaluating the whole selector.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// One term of a [`LabelSelector`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelTerm {
    /// `key=value` or `key==value`.
    Eq(String, String),
    /// `key!=value` (also true when the label is absent).
    NotEq(String, String),
    /// `key in (a,b)`.
    In(String, Vec<String>),
    /// `key notin (a,b)` (also true when the label is absent).
    NotIn(String, Vec<String>),
    /// `key`: the label is present.
    Exists(String),
    /// `!key`: the label is absent.
    NotExists(String),
}

impl LabelTerm {
    fn matches(&self, labels: &BTreeMap<Arc<str>, Arc<str>>) -> bool {
        let get = |key: &str| labels.get(key).map(|v| &**v);
        match self {
            LabelTerm::Eq(k, v) => get(k) == Some(v.as_str()),
            LabelTerm::NotEq(k, v) => get(k) != Some(v.as_str()),
            LabelTerm::In(k, vs) => get(k).is_some_and(|x| vs.iter().any(|v| v == x)),
            LabelTerm::NotIn(k, vs) => get(k).is_none_or(|x| vs.iter().all(|v| v != x)),
            LabelTerm::Exists(k) => get(k).is_some(),
            LabelTerm::NotExists(k) => get(k).is_none(),
        }
    }
}

/// A parsed label selector. The empty selector matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelSelector {
    terms: Vec<LabelTerm>,
}

/// Why a selector did not parse.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid label selector term `{term}`: {reason}")]
pub struct SelectorError {
    /// The offending term.
    pub term: String,
    /// What is wrong with it.
    pub reason: &'static str,
}

impl LabelSelector {
    /// Parses `input` (for example `app=web,tier!=db,env in (prod,staging),!canary`).
    ///
    /// # Errors
    ///
    /// [`SelectorError`] for an empty key, an unbalanced parenthesis or an empty set.
    pub fn parse(input: &str) -> Result<Self, SelectorError> {
        let terms = split_terms(input)
            .into_iter()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(parse_term)
            .collect::<Result<_, _>>()?;
        Ok(Self { terms })
    }

    /// A selector from already-built terms.
    pub fn from_terms(terms: Vec<LabelTerm>) -> Self {
        Self { terms }
    }

    /// The terms, in input order.
    pub fn terms(&self) -> &[LabelTerm] {
        &self.terms
    }

    /// Whether there are no terms (matches everything).
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Whether `labels` satisfy every term.
    pub fn matches(&self, labels: &BTreeMap<Arc<str>, Arc<str>>) -> bool {
        self.terms.iter().all(|t| t.matches(labels))
    }

    /// The `key=value` terms, which the store can answer from its label index.
    pub(crate) fn equalities(&self) -> impl Iterator<Item = (&str, &str)> {
        self.terms.iter().filter_map(|t| match t {
            LabelTerm::Eq(k, v) => Some((k.as_str(), v.as_str())),
            _ => None,
        })
    }
}

impl fmt::Display for LabelSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, term) in self.terms.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            match term {
                LabelTerm::Eq(k, v) => write!(f, "{k}={v}")?,
                LabelTerm::NotEq(k, v) => write!(f, "{k}!={v}")?,
                LabelTerm::In(k, vs) => write!(f, "{k} in ({})", vs.join(","))?,
                LabelTerm::NotIn(k, vs) => write!(f, "{k} notin ({})", vs.join(","))?,
                LabelTerm::Exists(k) => f.write_str(k)?,
                LabelTerm::NotExists(k) => write!(f, "!{k}")?,
            }
        }
        Ok(())
    }
}

/// Splits on commas outside parentheses.
fn split_terms(input: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in input.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&input[start..]);
    out
}

fn parse_term(term: &str) -> Result<LabelTerm, SelectorError> {
    let err = |reason| SelectorError {
        term: term.to_owned(),
        reason,
    };
    let key = |k: &str| {
        let k = k.trim();
        if k.is_empty() || k.contains(char::is_whitespace) {
            Err(err("expected a label key"))
        } else {
            Ok(k.to_owned())
        }
    };
    if let Some(rest) = term.strip_prefix('!') {
        return Ok(LabelTerm::NotExists(key(rest)?));
    }
    if let Some((k, v)) = term.split_once("!=") {
        return Ok(LabelTerm::NotEq(key(k)?, v.trim().to_owned()));
    }
    if let Some((k, v)) = term.split_once("==").or_else(|| term.split_once('=')) {
        return Ok(LabelTerm::Eq(key(k)?, v.trim().to_owned()));
    }
    for (op, negated) in [(" notin ", true), (" in ", false)] {
        if let Some((k, set)) = term.split_once(op) {
            let set = set.trim();
            let inner = set
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .ok_or_else(|| err("expected a parenthesised set"))?;
            let values: Vec<String> = inner
                .split(',')
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .collect();
            if values.is_empty() {
                return Err(err("empty set"));
            }
            let k = key(k)?;
            return Ok(if negated {
                LabelTerm::NotIn(k, values)
            } else {
                LabelTerm::In(k, values)
            });
        }
    }
    if term.contains(['(', ')']) {
        return Err(err("unbalanced parenthesis"));
    }
    Ok(LabelTerm::Exists(key(term)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<Arc<str>, Arc<str>> {
        pairs
            .iter()
            .map(|(k, v)| (Arc::from(*k), Arc::from(*v)))
            .collect()
    }

    #[test]
    fn parses_every_term_kind() {
        let s = LabelSelector::parse("app=web, tier==fe,env!=dev,zone in (a, b),x notin (y),k,!c")
            .unwrap();
        assert_eq!(
            s.terms(),
            &[
                LabelTerm::Eq("app".into(), "web".into()),
                LabelTerm::Eq("tier".into(), "fe".into()),
                LabelTerm::NotEq("env".into(), "dev".into()),
                LabelTerm::In("zone".into(), vec!["a".into(), "b".into()]),
                LabelTerm::NotIn("x".into(), vec!["y".into()]),
                LabelTerm::Exists("k".into()),
                LabelTerm::NotExists("c".into()),
            ]
        );
        assert_eq!(
            s.to_string(),
            "app=web,tier=fe,env!=dev,zone in (a,b),x notin (y),k,!c"
        );
        assert_eq!(s.equalities().count(), 2);
    }

    #[test]
    fn matches_like_kubectl() {
        let l = labels(&[("app", "web"), ("zone", "a")]);
        let ok = |s: &str| LabelSelector::parse(s).unwrap().matches(&l);
        assert!(ok(""));
        assert!(ok("app=web"));
        assert!(!ok("app=db"));
        assert!(ok("env!=prod"), "absent label satisfies !=");
        assert!(ok("zone in (a,b)"));
        assert!(!ok("env in (a)"));
        assert!(ok("env notin (a)"));
        assert!(!ok("zone notin (a)"));
        assert!(ok("app,!env"));
        assert!(!ok("app,zone=b"));
    }

    #[test]
    fn rejects_malformed_terms() {
        assert!(LabelSelector::parse("=web").is_err());
        assert!(LabelSelector::parse("zone in a").is_err());
        assert!(LabelSelector::parse("zone in ()").is_err());
        assert!(LabelSelector::parse("zone (").is_err());
        assert!(LabelSelector::parse("!").is_err());
    }
}
