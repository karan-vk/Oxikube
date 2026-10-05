//! Parsing of the structured `style` entries: `players`, `accents` and `syntax`.

use super::report::{ImportDiagnostic, ImportReport};
use crate::color::parse_color;
use crate::tokens::{FontStyle, PlayerColor, SyntaxStyle, SyntaxTheme};
use gpui::Hsla;
use serde_json::Value;

/// Reads a colour at `key`; reports a diagnostic and returns `None` when it is not one.
/// `null` (Zed's "unspecified") is silently `None`.
pub(super) fn color_at(
    value: &Value,
    theme: &str,
    key: &str,
    report: &mut ImportReport,
) -> Option<Hsla> {
    match value {
        Value::Null => None,
        Value::String(text) => match parse_color(text) {
            Ok(color) => Some(color),
            Err(message) => {
                report.diagnostics.push(ImportDiagnostic::InvalidColor {
                    theme: theme.to_owned(),
                    key: key.to_owned(),
                    value: text.clone(),
                    message,
                });
                None
            }
        },
        _ => {
            report.diagnostics.push(ImportDiagnostic::InvalidValue {
                theme: theme.to_owned(),
                key: key.to_owned(),
                expected: "a colour string like \"#rrggbbaa\"",
            });
            None
        }
    }
}

fn expected(report: &mut ImportReport, theme: &str, key: &str, expected: &'static str) {
    report.diagnostics.push(ImportDiagnostic::InvalidValue {
        theme: theme.to_owned(),
        key: key.to_owned(),
        expected,
    });
}

/// `players: [{ cursor, background, selection }]`. A missing part falls back to the cursor
/// colour (selection at 24 % alpha). Entries that are not objects are reported and skipped.
pub(super) fn players(
    value: &Value,
    theme: &str,
    report: &mut ImportReport,
) -> Option<Vec<PlayerColor>> {
    let Some(items) = value.as_array() else {
        expected(report, theme, "players", "an array");
        return None;
    };
    let mut players = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(object) = item.as_object() else {
            expected(report, theme, &format!("players[{index}]"), "an object");
            continue;
        };
        let mut part = |name: &str| {
            object
                .get(name)
                .and_then(|v| color_at(v, theme, &format!("players[{index}].{name}"), report))
        };
        let cursor = part("cursor");
        let background = part("background");
        let selection = part("selection");
        let Some(base) = cursor.or(background).or(selection) else {
            continue;
        };
        players.push(PlayerColor {
            cursor: cursor.unwrap_or(base),
            background: background.unwrap_or(base),
            selection: selection.unwrap_or_else(|| base.opacity(0.24)),
        });
    }
    Some(players)
}

/// `accents: ["#rrggbbaa", ..]`; invalid entries are reported and skipped.
pub(super) fn accents(value: &Value, theme: &str, report: &mut ImportReport) -> Option<Vec<Hsla>> {
    let Some(items) = value.as_array() else {
        expected(report, theme, "accents", "an array");
        return None;
    };
    Some(
        items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| color_at(item, theme, &format!("accents[{index}]"), report))
            .collect(),
    )
}

/// `syntax: { "keyword": { color, font_style, font_weight }, .. }`, merged over `base`.
pub(super) fn syntax(
    value: &Value,
    theme: &str,
    base: &SyntaxTheme,
    report: &mut ImportReport,
) -> Option<SyntaxTheme> {
    let Some(entries) = value.as_object() else {
        expected(report, theme, "syntax", "an object");
        return None;
    };
    let mut merged = base.clone();
    for (name, entry) in entries {
        let key = format!("syntax.{name}");
        let Some(entry) = entry.as_object() else {
            expected(report, theme, &key, "an object");
            continue;
        };
        let color = entry
            .get("color")
            .and_then(|v| color_at(v, theme, &format!("{key}.color"), report));
        let font_style = match entry.get("font_style").and_then(Value::as_str) {
            Some("normal") => Some(FontStyle::Normal),
            Some("italic") => Some(FontStyle::Italic),
            Some("oblique") => Some(FontStyle::Oblique),
            Some(_) => {
                expected(
                    report,
                    theme,
                    &format!("{key}.font_style"),
                    "normal, italic or oblique",
                );
                None
            }
            None => None,
        };
        let font_weight = entry
            .get("font_weight")
            .and_then(Value::as_f64)
            .map(|weight| weight as f32);
        // Zed treats an entry as a patch over the default syntax theme: keep what it omits.
        let existing = merged.styles.get(name).copied().unwrap_or_default();
        merged.styles.insert(
            name.clone(),
            SyntaxStyle {
                color: color.or(existing.color),
                font_style: font_style.or(existing.font_style),
                font_weight: font_weight.or(existing.font_weight),
            },
        );
    }
    Some(merged)
}
