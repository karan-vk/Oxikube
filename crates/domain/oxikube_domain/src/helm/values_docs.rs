//! [`ValuesDocs`]: the documentation of a chart's values, for the values editor's hover and
//! completion (E16-S09).
//!
//! Two sources, both optional:
//!
//! * the comments of the chart's default `values.yaml` (`helm show values`). The
//!   [helm-docs](https://github.com/norwoodj/helm-docs) convention wins where it is used: a
//!   `# -- description` block right above a key (continued by more `#` lines, an optional
//!   `(type)` right after `--`, and `# @default -- text`), or the old form
//!   `# path.to.key -- description` anywhere. Otherwise the plain comment lines right above a
//!   key are its description. A blank line ends a comment block.
//! * the chart's `values.schema.json` ([`merge_schema`](ValuesDocs::merge_schema)): its
//!   `description`, `type` and `enum` fill what the comments left out.
//!
//! Every key of the defaults is recorded, documented or not, so completion can offer it.
//! Lexical and line-based: a 200 KB values file is one pass, with no YAML parse.

use std::collections::BTreeMap;

use serde_json::Value;

/// What is known about one key of the values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyDoc {
    /// The description (comment or schema text), possibly several lines.
    pub description: String,
    /// The type (`(type)` of helm-docs, the schema's `type`, else read off the default).
    pub type_hint: Option<String>,
    /// The default: `@default` of helm-docs, else the literal value in `values.yaml`.
    pub default: Option<String>,
    /// The allowed values, from the schema's `enum`.
    pub allowed: Vec<String>,
}

/// The documentation of a chart's values, by dotted key path (`image.tag`). See the
/// [module docs](self).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ValuesDocs {
    keys: BTreeMap<String, KeyDoc>,
}

/// A comment block being collected above the next key.
#[derive(Default)]
struct Pending {
    helm_docs: bool,
    type_hint: Option<String>,
    default: Option<String>,
    lines: Vec<String>,
}

impl Pending {
    fn take(&mut self) -> Pending {
        std::mem::take(self)
    }
}

impl ValuesDocs {
    /// Reads the keys and comments of a `values.yaml`.
    pub fn from_values_yaml(text: &str) -> Self {
        let mut docs = Self::default();
        // (indent, key) of the maps the current line is inside.
        let mut stack: Vec<(usize, String)> = Vec::new();
        let mut pending = Pending::default();
        // The indent of a key whose value is a block scalar (`key: |`), or of a list item's `-`:
        // deeper lines are its text or the item's own keys, not keys of the values' tree.
        let mut skip_deeper: Option<usize> = None;
        for line in text.lines() {
            let indent = line.len() - line.trim_start().len();
            let body = line.trim();
            if let Some(level) = skip_deeper {
                if body.is_empty() || indent > level {
                    continue;
                }
                skip_deeper = None;
            }
            if body.is_empty() {
                pending = Pending::default();
                continue;
            }
            if let Some(comment) = body.strip_prefix('#') {
                docs.comment(comment, &mut pending);
                continue;
            }
            // A list item: its keys (on this line and the deeper ones that continue it) belong to
            // an element, not to the values' own tree.
            if body.starts_with('-') {
                pending = Pending::default();
                skip_deeper = Some(indent);
                continue;
            }
            let Some((key, value)) = split_key(body) else {
                pending = Pending::default();
                continue;
            };
            while stack.last().is_some_and(|(level, _)| *level >= indent) {
                stack.pop();
            }
            let path = stack
                .iter()
                .map(|(_, k)| k.as_str())
                .chain(std::iter::once(key.as_str()))
                .collect::<Vec<_>>()
                .join(".");
            docs.record(&path, value, pending.take());
            match value {
                "" => stack.push((indent, key)),
                v if is_block_scalar(v) => skip_deeper = Some(indent),
                _ => {}
            }
        }
        docs
    }

    fn comment(&mut self, comment: &str, pending: &mut Pending) {
        let text = comment.strip_prefix(' ').unwrap_or(comment).trim_end();
        let trimmed = text.trim_start();
        if let Some(rest) = trimmed.strip_prefix("@default --") {
            pending.default = Some(rest.trim().to_owned());
        } else if let Some(rest) = trimmed
            .strip_prefix("--")
            .filter(|r| r.is_empty() || r.starts_with(' '))
        {
            // A new helm-docs block replaces whatever plain comment came before.
            let rest = rest.trim();
            let (type_hint, rest) = match rest.strip_prefix('(').and_then(|r| r.split_once(')')) {
                Some((ty, rest)) => (Some(ty.trim().to_owned()), rest.trim()),
                None => (None, rest),
            };
            *pending = Pending {
                helm_docs: true,
                type_hint,
                default: None,
                lines: vec![rest.to_owned()],
            };
        } else if let Some((path, rest)) = trimmed.split_once(" -- ").filter(|(p, _)| is_path(p)) {
            // `# path.to.key -- description`: documents that key wherever it is written.
            let doc = self.keys.entry(path.to_owned()).or_default();
            doc.description = rest.trim().to_owned();
        } else {
            pending.lines.push(text.to_owned());
        }
    }

    fn record(&mut self, path: &str, value: &str, pending: Pending) {
        let doc = self.keys.entry(path.to_owned()).or_default();
        let description = pending.lines.join("\n").trim().to_owned();
        if !description.is_empty() && (pending.helm_docs || doc.description.is_empty()) {
            doc.description = description;
        }
        if pending.type_hint.is_some() {
            doc.type_hint = pending.type_hint;
        }
        let literal = literal(value);
        doc.default = pending
            .default
            .or(doc.default.take())
            .or_else(|| literal.clone());
        if doc.type_hint.is_none() {
            doc.type_hint = literal
                .as_deref()
                .map(infer_type)
                .or_else(|| value.is_empty().then(|| "object".to_owned()));
        }
    }

    /// Fills what the comments left out from a `values.schema.json`: descriptions, types and the
    /// allowed values of `enum`s, and the keys the schema declares but the defaults lack.
    pub fn merge_schema(&mut self, schema: &Value) {
        self.merge_properties(schema, "", 0);
    }

    fn merge_properties(&mut self, schema: &Value, prefix: &str, depth: usize) {
        const MAX_DEPTH: usize = 32;
        let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
            return;
        };
        if depth > MAX_DEPTH {
            return;
        }
        for (key, property) in properties {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            let doc = self.keys.entry(path.clone()).or_default();
            if doc.description.is_empty()
                && let Some(description) = property.get("description").and_then(Value::as_str)
            {
                description.trim().clone_into(&mut doc.description);
            }
            if let Some(ty) = schema_type(property) {
                doc.type_hint = Some(ty);
            }
            if let Some(values) = property.get("enum").and_then(Value::as_array) {
                doc.allowed = values.iter().map(scalar_text).collect();
            }
            self.merge_properties(property, &path, depth + 1);
        }
    }

    /// The documentation of `path` (`image.tag`).
    pub fn get(&self, path: &str) -> Option<&KeyDoc> {
        self.keys.get(path)
    }

    /// The direct children of `parent` (`""` for the top level), by key, in key order.
    pub fn children<'a>(&'a self, parent: &str) -> impl Iterator<Item = (&'a str, &'a KeyDoc)> {
        let prefix = if parent.is_empty() {
            String::new()
        } else {
            format!("{parent}.")
        };
        self.keys
            .range(prefix.clone()..)
            .take_while(move |(path, _)| path.starts_with(&prefix))
            .filter_map(move |(path, doc)| {
                let rest = &path[prefix_len(parent)..];
                (!rest.contains('.')).then_some((rest, doc))
            })
    }

    /// How many keys are known.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// `true` when no key is known.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The hover text of `path` as Markdown: the path and type, the description, the default
    /// and the allowed values. `None` for an unknown key.
    pub fn hover_markdown(&self, path: &str) -> Option<String> {
        let doc = self.get(path)?;
        let mut out = format!("**{path}**");
        if let Some(ty) = &doc.type_hint {
            out.push_str(&format!(" `{ty}`"));
        }
        if !doc.description.is_empty() {
            out.push_str("\n\n");
            out.push_str(&doc.description);
        }
        if let Some(default) = doc.default.as_deref().filter(|d| !d.is_empty()) {
            out.push_str(&format!("\n\nDefault: `{default}`"));
        }
        if !doc.allowed.is_empty() {
            out.push_str(&format!("\n\nOne of: {}", doc.allowed.join(", ")));
        }
        Some(out)
    }
}

fn prefix_len(parent: &str) -> usize {
    if parent.is_empty() {
        0
    } else {
        parent.len() + 1
    }
}

/// `key: value` of a mapping line: the key (unquoted) and the value text, inline comment cut.
fn split_key(body: &str) -> Option<(String, &str)> {
    let (key, rest) = if let Some(quoted) = body.strip_prefix('"') {
        let (key, rest) = quoted.split_once('"')?;
        (key, rest.strip_prefix(':')?)
    } else if let Some(quoted) = body.strip_prefix('\'') {
        let (key, rest) = quoted.split_once('\'')?;
        (key, rest.strip_prefix(':')?)
    } else {
        let colon = body
            .find(": ")
            .or_else(|| body.ends_with(':').then(|| body.len() - 1))?;
        (&body[..colon], &body[colon + 1..])
    };
    let key = key.trim();
    if key.is_empty() || (key.contains(char::is_whitespace) && !body.starts_with(['"', '\''])) {
        return None;
    }
    let value = rest.trim();
    let value = match value.find(" #") {
        Some(at) if !value.starts_with(['"', '\'']) => value[..at].trim_end(),
        _ => value,
    };
    Some((key.to_owned(), value))
}

fn is_block_scalar(value: &str) -> bool {
    value.starts_with('|') || value.starts_with('>')
}

/// A dotted key path (no spaces): the `path` of `# path -- description`.
fn is_path(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The default shown for a value, `None` when the key opens a map or a block.
fn literal(value: &str) -> Option<String> {
    (!value.is_empty() && !is_block_scalar(value)).then(|| value.to_owned())
}

fn infer_type(value: &str) -> String {
    let ty = match value {
        "true" | "false" => "bool",
        "null" | "~" => "string",
        v if v.starts_with('{') => "object",
        v if v.starts_with('[') => "list",
        v if v.parse::<i64>().is_ok() => "int",
        v if v.parse::<f64>().is_ok() => "float",
        _ => "string",
    };
    ty.to_owned()
}

fn schema_type(property: &Value) -> Option<String> {
    match property.get("type")? {
        Value::String(ty) => Some(ty.clone()),
        Value::Array(types) => Some(
            types
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" | "),
        ),
        _ => None,
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}
