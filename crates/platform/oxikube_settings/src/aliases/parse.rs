//! Reading `aliases.json`: JSON with comments to [`UserAlias`]es and [`AliasDiagnostic`]s.

use std::collections::HashMap;
use std::fmt;

use oxikube_domain::AliasTarget;
use oxikube_domain::ids::Gvr;
use serde::Deserialize as _;
use serde_json::{Map, Value};

use super::lines::top_level_keys;

/// Longest alias name accepted.
pub const MAX_ALIAS_NAME_LEN: usize = 63;

/// Characters the jump bar's grammar gives a meaning of their own (`/re`, `@ctx`, `k=v`,
/// `a,b`), so a name containing one could never be typed as an alias.
const RESERVED: &[char] = &['/', '@', '=', ','];

/// One alias read from the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAlias {
    /// The alias, lower-cased.
    pub name: String,
    /// Where it leads.
    pub target: AliasTarget,
    /// The 1-based line of its key in the file.
    pub line: usize,
}

/// A problem with the file or one of its entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasDiagnostic {
    /// The 1-based line (the parser's for a syntax error, the entry's key otherwise); `None`
    /// when the problem has no position (an unreadable file).
    pub line: Option<usize>,
    /// The alias the problem is about, as written; `None` for a whole-file problem.
    pub alias: Option<String>,
    /// What is wrong, in a sentence.
    pub message: String,
}

impl AliasDiagnostic {
    /// A problem with the file as a whole.
    pub fn file(line: Option<usize>, message: impl Into<String>) -> Self {
        Self {
            line,
            alias: None,
            message: message.into(),
        }
    }

    fn entry(line: usize, alias: &str, message: impl Into<String>) -> Self {
        Self {
            line: Some(line),
            alias: Some(alias.to_owned()),
            message: message.into(),
        }
    }
}

impl fmt::Display for AliasDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("aliases.json")?;
        if let Some(line) = self.line {
            write!(f, " line {line}")?;
        }
        if let Some(alias) = &self.alias {
            write!(f, " (`{alias}`)")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// What a readable file contained.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedAliases {
    /// The valid entries, in file order.
    pub aliases: Vec<UserAlias>,
    /// What was skipped or looks wrong.
    pub diagnostics: Vec<AliasDiagnostic>,
}

/// Parses the text of an `aliases.json`. Blank text is no aliases.
///
/// # Errors
///
/// A problem with the file as a whole (not JSON, not an object): the caller keeps the aliases it
/// had. Bad entries are not errors; they are skipped and listed in
/// [`ParsedAliases::diagnostics`].
pub fn parse_aliases(text: &str) -> Result<ParsedAliases, AliasDiagnostic> {
    if text.trim().is_empty() {
        return Ok(ParsedAliases::default());
    }
    let mut deserializer = serde_json_lenient::Deserializer::from_str(text);
    let value = Value::deserialize(&mut deserializer)
        .map_err(|e| AliasDiagnostic::file(Some(e.line()), format!("is not valid JSON: {e}")))?;
    deserializer
        .end()
        .map_err(|e| AliasDiagnostic::file(Some(e.line()), format!("is not valid JSON: {e}")))?;
    let Value::Object(entries) = value else {
        return Err(AliasDiagnostic::file(
            Some(1),
            "must be a JSON object that maps each alias to its target",
        ));
    };
    Ok(read_entries(text, &entries))
}

fn read_entries(text: &str, entries: &Map<String, Value>) -> ParsedAliases {
    let mut parsed = ParsedAliases::default();
    // Lines of each key; a repeated key lists them all, the last one is the value `entries` holds.
    let mut lines: HashMap<String, Vec<usize>> = HashMap::new();
    for (key, line) in top_level_keys(text) {
        lines.entry(key).or_default().push(line);
    }
    // Entries in the order they are written (a repeated key is at its last line).
    let mut written: Vec<(&String, &Value, usize)> = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        if key == "$schema" {
            continue;
        }
        let key_lines = lines.get(key).map_or(&[][..], Vec::as_slice);
        let line = key_lines.last().copied().unwrap_or(1);
        if key_lines.len() > 1 {
            parsed.diagnostics.push(AliasDiagnostic::entry(
                key_lines[0],
                key,
                format!("is defined again on line {line}; the later one is used"),
            ));
        }
        written.push((key, value, line));
    }
    written.sort_by_key(|(_, _, line)| *line);

    // First line of each lower-cased name, to report names that differ only in case.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (key, value, line) in written {
        if let Some(problem) = alias_name_problem(key) {
            parsed
                .diagnostics
                .push(AliasDiagnostic::entry(line, key, problem));
            continue;
        }
        let name = key.to_ascii_lowercase();
        if let Some(first) = seen.get(&name) {
            parsed.diagnostics.push(AliasDiagnostic::entry(
                line,
                key,
                format!("is the same alias as the one on line {first} (names ignore case); this one is used"),
            ));
            // The later definition wins, as for a repeated key.
            parsed.aliases.retain(|a| a.name != name);
        }
        seen.insert(name.clone(), line);
        match read_target(value) {
            Ok(target) => parsed.aliases.push(UserAlias { name, target, line }),
            Err(problem) => parsed
                .diagnostics
                .push(AliasDiagnostic::entry(line, key, problem)),
        }
    }
    parsed.diagnostics.sort_by_key(|d| d.line);
    parsed
}

/// Why `name` cannot be an alias, or `None` when it can.
pub fn alias_name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("an alias needs a name".to_owned());
    }
    if name.len() > MAX_ALIAS_NAME_LEN {
        return Some(format!("is longer than {MAX_ALIAS_NAME_LEN} characters"));
    }
    if name.chars().any(char::is_whitespace) {
        return Some("a name cannot contain spaces".to_owned());
    }
    if let Some(c) = name
        .chars()
        .find(|c| RESERVED.contains(c) || c.is_control())
    {
        return Some(format!(
            "a name cannot contain `{}`: the jump bar reads it as something else",
            c.escape_default()
        ));
    }
    None
}

fn read_target(value: &Value) -> Result<AliasTarget, String> {
    match value {
        Value::String(text) => read_text_target(text),
        Value::Object(fields) => read_object_target(fields),
        other => Err(format!(
            "expected a resource (\"apps/v1/deployments\"), a command line (\"pod fred app=blee\") or an object, found {}",
            kind_of(other)
        )),
    }
}

/// `"group/version/plural"` is a resource; any other text is a command line.
fn read_text_target(text: &str) -> Result<AliasTarget, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the target is empty".to_owned());
    }
    if text.contains('/') && !text.contains(char::is_whitespace) {
        return read_gvr(text);
    }
    let mut words = text.split_whitespace();
    let name = words.next().unwrap_or(text);
    Ok(AliasTarget::command(name, words.map(str::to_owned)))
}

fn read_gvr(text: &str) -> Result<AliasTarget, String> {
    text.parse::<Gvr>().map(AliasTarget::Gvr).map_err(|_| {
        format!(
            "`{text}` is not a resource: write group/version/plural (`apps/v1/deployments`), or `v1/pods` for the core group"
        )
    })
}

fn read_object_target(fields: &Map<String, Value>) -> Result<AliasTarget, String> {
    if let Some(unknown) = fields
        .keys()
        .find(|k| !matches!(k.as_str(), "gvr" | "command" | "args"))
    {
        return Err(format!(
            "unknown field `{unknown}`: use `gvr`, or `command` with optional `args`"
        ));
    }
    match (fields.get("gvr"), fields.get("command")) {
        (Some(_), Some(_)) => Err("use either `gvr` or `command`, not both".to_owned()),
        (None, None) => Err("an object needs `gvr` or `command`".to_owned()),
        (Some(gvr), None) => {
            if fields.contains_key("args") {
                return Err("`args` only goes with `command`".to_owned());
            }
            match gvr {
                Value::String(text) => read_gvr(text.trim()),
                other => Err(format!("`gvr` must be a string, found {}", kind_of(other))),
            }
        }
        (None, Some(command)) => {
            let Value::String(name) = command else {
                return Err(format!(
                    "`command` must be a string, found {}",
                    kind_of(command)
                ));
            };
            let name = name.trim();
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err("`command` is one word; put the rest in `args`".to_owned());
            }
            let args = match fields.get("args") {
                None => Vec::new(),
                Some(Value::Array(items)) => items
                    .iter()
                    .map(|item| match item {
                        Value::String(s) => Ok(s.clone()),
                        other => Err(format!("`args` must be strings, found {}", kind_of(other))),
                    })
                    .collect::<Result<_, _>>()?,
                Some(other) => {
                    return Err(format!(
                        "`args` must be a list of strings, found {}",
                        kind_of(other)
                    ));
                }
            };
            Ok(AliasTarget::command(name, args))
        }
    }
}

fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gvr(group: &str, version: &str, resource: &str) -> AliasTarget {
        AliasTarget::Gvr(Gvr::new(group, version, resource))
    }

    #[test]
    fn every_documented_form_parses() {
        let parsed = parse_aliases(
            r#"
            // my names
            {
              "$schema": "https://example.invalid/aliases.schema.json",
              "prodpods": "v1/pods",
              "Dep": { "gvr": "apps/v1/deployments" },
              "fred": "pod fred app=blee",
              "blee": { "command": "pod", "args": ["fred", "app=blee"] },
              "ctx2": "ctx",
              "w": "example.io/v1/widgets", /* trailing comma next */
            }"#,
        )
        .unwrap();
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let by_name: HashMap<_, _> = parsed
            .aliases
            .iter()
            .map(|a| (a.name.as_str(), (&a.target, a.line)))
            .collect();
        assert_eq!(by_name["prodpods"], (&gvr("", "v1", "pods"), 5));
        assert_eq!(by_name["dep"].0, &gvr("apps", "v1", "deployments"));
        let fred = AliasTarget::command("pod", ["fred".to_owned(), "app=blee".to_owned()]);
        assert_eq!(by_name["fred"].0, &fred);
        assert_eq!(by_name["blee"].0, &fred);
        assert_eq!(by_name["ctx2"].0, &AliasTarget::command("ctx", []));
        assert_eq!(by_name["w"].0, &gvr("example.io", "v1", "widgets"));
        let order: Vec<_> = parsed.aliases.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            order,
            ["prodpods", "dep", "fred", "blee", "ctx2", "w"],
            "file order"
        );
    }

    #[test]
    fn blank_text_is_no_aliases_and_a_comment_only_file_too() {
        assert_eq!(parse_aliases("  \n").unwrap(), ParsedAliases::default());
        assert_eq!(
            parse_aliases("// nothing\n{}").unwrap(),
            ParsedAliases::default()
        );
    }

    #[test]
    fn a_bad_entry_is_reported_with_its_line_and_the_rest_load() {
        let parsed = parse_aliases(
            "{\n  \"ok\": \"v1/pods\",\n  \"bad gvr\": \"v1/\",\n  \"num\": 3,\n  \"two\": {\"gvr\": \"v1/pods\", \"command\": \"x\"},\n  \"typo\": {\"cmd\": \"x\"},\n  \"also ok\": \"pod\"\n}",
        )
        .unwrap();
        let names: Vec<_> = parsed.aliases.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["ok"]);
        let lines: Vec<_> = parsed.diagnostics.iter().map(|d| d.line).collect();
        assert_eq!(lines, [Some(3), Some(4), Some(5), Some(6), Some(7)]);
        assert!(
            parsed.diagnostics[0].message.contains("spaces"),
            "{:?}",
            parsed.diagnostics[0]
        );
        assert!(parsed.diagnostics[1].message.contains("a number"));
        assert!(parsed.diagnostics[2].message.contains("either"));
        assert!(
            parsed.diagnostics[3]
                .message
                .contains("unknown field `cmd`")
        );
        assert_eq!(
            parsed.diagnostics[0].to_string(),
            "aliases.json line 3 (`bad gvr`): a name cannot contain spaces"
        );
    }

    #[test]
    fn a_resource_that_does_not_parse_says_how_to_write_one() {
        let parsed = parse_aliases("{\"x\": \"apps//deployments\", \"y\": {\"gvr\": 4}}").unwrap();
        assert!(parsed.aliases.is_empty());
        assert!(
            parsed.diagnostics[0]
                .message
                .contains("group/version/plural")
        );
        assert!(parsed.diagnostics[1].message.contains("must be a string"));
    }

    #[test]
    fn names_the_jump_bar_would_misread_are_refused() {
        for name in ["a/b", "@ctx", "k=v", "a,b", "with space", ""] {
            assert!(alias_name_problem(name).is_some(), "{name:?}");
        }
        assert!(alias_name_problem(&"x".repeat(MAX_ALIAS_NAME_LEN + 1)).is_some());
        for name in ["po", "my-pods", "my.pods", "pods:prod", "Ünï"] {
            assert_eq!(alias_name_problem(name), None, "{name}");
        }
    }

    #[test]
    fn a_repeated_alias_reports_the_earlier_line_and_uses_the_later() {
        let parsed = parse_aliases(
            "{\n \"a\": \"v1/pods\",\n \"A\": \"v1/nodes\",\n \"a\": \"v1/events\"\n}",
        )
        .unwrap();
        // `serde_json` keeps the last `a` and `A` as separate keys.
        let a: Vec<_> = parsed.aliases.iter().filter(|x| x.name == "a").collect();
        assert_eq!(a.len(), 1, "one alias per name");
        assert_eq!(a[0].target, gvr("", "v1", "events"));
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.line == Some(2) && d.message.contains("defined again on line 4"))
        );
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|d| d.message.contains("ignore case"))
        );
    }

    #[test]
    fn whole_file_problems_are_errors_with_a_line() {
        let err = parse_aliases("{\n  \"a\": \n}").unwrap_err();
        assert_eq!(err.line, Some(3));
        assert!(err.message.contains("not valid JSON"));
        assert!(
            parse_aliases("[1]")
                .unwrap_err()
                .message
                .contains("JSON object")
        );
        assert!(parse_aliases("{} {}").is_err());
    }
}
