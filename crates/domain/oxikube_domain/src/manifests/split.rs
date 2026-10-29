//! Splitting manifest text into YAML documents, remembering where each starts.

/// One document's text and where it sits in the input.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Chunk {
    /// 1-based line of the first line of `text`.
    pub(super) first_line: usize,
    /// 1-based line of the first line that is not blank, a comment or a directive.
    pub(super) content_line: usize,
    /// The lines, joined with `\n` (CRs dropped).
    pub(super) text: String,
}

/// Whether `line` is a document marker: `marker` at column 0 followed by the end of the line or
/// white space. Returns what follows the marker.
fn marker_rest<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(marker)?;
    (rest.is_empty() || rest.starts_with([' ', '\t'])).then_some(rest)
}

/// A line that carries no content of its own.
fn is_blank(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty() || trimmed.starts_with('#') || line.starts_with('%')
}

/// Splits `text` at `---` and `...` lines. Chunks without content are dropped.
pub(super) fn split(text: &str) -> Vec<Chunk> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut first_line = 1;
    let mut content_line = None;
    let mut finish = |current: &mut String, first_line: usize, content_line: &mut Option<usize>| {
        if let Some(content_line) = content_line.take() {
            chunks.push(Chunk {
                first_line,
                content_line,
                text: std::mem::take(current),
            });
        }
        current.clear();
    };
    for (index, raw) in text.split('\n').enumerate() {
        let number = index + 1;
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = marker_rest(line, "---") {
            finish(&mut current, first_line, &mut content_line);
            first_line = number;
            let rest = rest.trim_start();
            if !rest.is_empty() && !rest.starts_with('#') {
                // `--- text`: the document starts on the marker's line.
                content_line = Some(number);
                current.push_str(rest);
                current.push('\n');
            } else {
                first_line = number + 1;
            }
            continue;
        }
        if marker_rest(line, "...").is_some() {
            finish(&mut current, first_line, &mut content_line);
            first_line = number + 1;
            continue;
        }
        if content_line.is_none() && !is_blank(line) {
            content_line = Some(number);
        }
        current.push_str(line);
        current.push('\n');
    }
    finish(&mut current, first_line, &mut content_line);
    chunks
}
