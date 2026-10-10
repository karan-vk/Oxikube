//! JSON paths into a YAML document: `spec.template.spec.containers[0].image`.

use std::fmt;
use std::str::FromStr;

/// One step of a [`JsonPath`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PathSegment {
    /// A mapping key (the decoded key text; a collection used as a key gives its source text).
    Key(Box<str>),
    /// A sequence index.
    Index(usize),
}

/// A path from a document's root to a node. The empty path is the root.
///
/// Displayed jq-style: plain keys joined by `.`, indexes as `[n]`, and keys that are not plain
/// identifiers (`app.kubernetes.io/name`, empty, quotes) as `["..."]`; the root displays as `.`.
/// [`FromStr`] reads the same notation back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JsonPath(pub Vec<PathSegment>);

/// A [`JsonPath`] in one document of a multi-document buffer (`---`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocPath {
    /// Zero-based document index.
    pub doc: usize,
    /// Path inside that document.
    pub path: JsonPath,
}

impl JsonPath {
    /// The root path.
    #[must_use]
    pub fn root() -> Self {
        Self::default()
    }

    /// The segments, root first.
    #[must_use]
    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }

    /// This path extended by a key.
    #[must_use]
    pub fn key(mut self, key: impl Into<Box<str>>) -> Self {
        self.0.push(PathSegment::Key(key.into()));
        self
    }

    /// This path extended by an index.
    #[must_use]
    pub fn index(mut self, index: usize) -> Self {
        self.0.push(PathSegment::Index(index));
        self
    }
}

fn is_plain_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'$')
}

impl fmt::Display for JsonPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return f.write_str(".");
        }
        for (i, segment) in self.0.iter().enumerate() {
            match segment {
                PathSegment::Index(n) => write!(f, "[{n}]")?,
                PathSegment::Key(k) if is_plain_key(k) => {
                    if i > 0 {
                        f.write_str(".")?;
                    }
                    f.write_str(k)?;
                }
                PathSegment::Key(k) => {
                    f.write_str("[\"")?;
                    for c in k.chars() {
                        if c == '"' || c == '\\' {
                            f.write_str("\\")?;
                        }
                        write!(f, "{c}")?;
                    }
                    f.write_str("\"]")?;
                }
            }
        }
        Ok(())
    }
}

impl fmt::Display for DocPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "doc {}: {}", self.doc, self.path)
    }
}

/// A string that is not a valid [`JsonPath`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathParseError {
    /// Byte offset of the problem in the input.
    pub at: usize,
}

impl fmt::Display for PathParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid JSON path at byte {}", self.at)
    }
}

impl std::error::Error for PathParseError {}

impl FromStr for JsonPath {
    type Err = PathParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = s.as_bytes();
        let mut segments = Vec::new();
        let mut i = 0;
        if s == "." {
            return Ok(Self::root());
        }
        while i < bytes.len() {
            match bytes[i] {
                b'.' if i > 0 && i + 1 < bytes.len() && bytes[i + 1] != b'[' => i += 1,
                b'[' => {
                    let (segment, next) = bracket(s, i)?;
                    segments.push(segment);
                    i = next;
                    continue;
                }
                _ if i == 0 => {}
                _ => return Err(PathParseError { at: i }),
            }
            let start = i;
            while i < bytes.len() && bytes[i] != b'.' && bytes[i] != b'[' {
                i += 1;
            }
            if i == start {
                return Err(PathParseError { at: i });
            }
            segments.push(PathSegment::Key(s[start..i].into()));
        }
        Ok(Self(segments))
    }
}

/// Parses `[n]` or `["key"]` starting at the `[` at `open`; returns the segment and the offset
/// after `]`.
fn bracket(s: &str, open: usize) -> Result<(PathSegment, usize), PathParseError> {
    let rest = &s[open + 1..];
    if let Some(quoted) = rest.strip_prefix('"') {
        let mut key = String::new();
        let mut escaped = false;
        for (j, c) in quoted.char_indices() {
            match c {
                _ if escaped => {
                    key.push(c);
                    escaped = false;
                }
                '\\' => escaped = true,
                '"' => {
                    let close = open + 2 + j + 1;
                    return match s.as_bytes().get(close) {
                        Some(b']') => Ok((PathSegment::Key(key.into()), close + 1)),
                        _ => Err(PathParseError { at: close }),
                    };
                }
                _ => key.push(c),
            }
        }
        return Err(PathParseError { at: s.len() });
    }
    let close = rest.find(']').ok_or(PathParseError { at: s.len() })?;
    let index = rest[..close]
        .parse()
        .map_err(|_| PathParseError { at: open + 1 })?;
    Ok((PathSegment::Index(index), open + 1 + close + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_and_parses_back() {
        let path = JsonPath::root()
            .key("spec")
            .key("containers")
            .index(0)
            .key("image");
        assert_eq!(path.to_string(), "spec.containers[0].image");
        assert_eq!("spec.containers[0].image".parse::<JsonPath>(), Ok(path));

        let odd = JsonPath::root()
            .key("metadata")
            .key("labels")
            .key("app.kubernetes.io/name")
            .key("q\"\\")
            .key("");
        let text = odd.to_string();
        assert_eq!(
            text,
            r#"metadata.labels["app.kubernetes.io/name"]["q\"\\"][""]"#
        );
        assert_eq!(text.parse::<JsonPath>(), Ok(odd));
    }

    #[test]
    fn root_and_leading_index() {
        assert_eq!(JsonPath::root().to_string(), ".");
        assert_eq!(".".parse::<JsonPath>(), Ok(JsonPath::root()));
        assert_eq!("".parse::<JsonPath>(), Ok(JsonPath::root()));
        assert_eq!("[2].a".parse(), Ok(JsonPath::root().index(2).key("a")));
        assert_eq!(JsonPath::root().index(2).key("a").to_string(), "[2].a");
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["a..b", "a.", "a[x]", "a[1", "a[\"x\"", "a[\"x\"x]", ".a"] {
            assert!(bad.parse::<JsonPath>().is_err(), "{bad}");
        }
    }

    #[test]
    fn doc_path_display() {
        let p = DocPath {
            doc: 1,
            path: "spec.replicas".parse().unwrap(),
        };
        assert_eq!(p.to_string(), "doc 1: spec.replicas");
    }
}
