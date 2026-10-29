//! The lines of the table: one per row, and under an expanded row one per line of its message.
//!
//! The table is a virtualised list, whose rows all have one height, so an expanded row cannot be
//! taller: its full message and field paths are lines of their own under it.

use oxikube_app::apply::ObjectStatus;

use super::row::Row;

/// The longest a detail line is, in characters, before it wraps.
const WRAP: usize = 110;

/// One line of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// The row at this index into the rows.
    Row(usize),
    /// A line of the expanded row at this index.
    Detail(usize, String),
}

/// The detail lines of `row`: the server's message in full, then the field paths it named and the
/// fields other managers own. Empty for a row that did not fail.
pub fn details(row: &Row) -> Vec<String> {
    let mut lines = Vec::new();
    match row.shown() {
        Some(ObjectStatus::Failed {
            reason,
            fields,
            conflicts,
        }) => {
            lines.extend(wrap(reason));
            lines.extend(fields.iter().map(|field| format!("field: {field}")));
            for conflict in conflicts {
                lines.push(match &conflict.manager {
                    Some(manager) => format!("conflict: {} (owned by {manager})", conflict.field),
                    None => format!("conflict: {}", conflict.field),
                });
            }
        }
        Some(ObjectStatus::Skipped { reason }) => lines.extend(wrap(reason)),
        _ => {}
    }
    lines
}

/// `text` cut into lines of at most [`WRAP`] characters, at spaces where it can.
fn wrap(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > WRAP {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn long_messages_wrap_at_spaces_and_short_ones_stay() {
        assert_eq!(wrap("short"), ["short"]);
        let long = "word ".repeat(60);
        let lines = wrap(&long);
        assert!(lines.len() >= 3);
        assert!(lines.iter().all(|line| line.chars().count() <= 110));
        assert_eq!(lines.join(" "), long.trim());
    }
}
