//! The one table that maps Zed syntax names onto the editor's capture names.
//!
//! The editor (gpui-component's tree-sitter highlighter) paints the capture names in
//! [`EDITOR_CAPTURES`]; a raw query capture such as `constant.builtin` is first reduced to the
//! longest of them that prefixes it ([`editor_capture`]). A Zed theme names its syntax styles the
//! same way, but a theme may leave a name out. [`resolve`] finds the style for a capture: the
//! theme's own entry, then its dotted ancestors (`string.escape` -> `string`), then the capture's
//! alias here (`boolean` -> `constant`). A capture nothing covers has no style and is painted
//! with the editor's plain text colour.
//!
//! The table lives here (not in `oxikube_editor`) because only `oxikube_ui` may speak
//! gpui-component's vocabulary, and the editor reads the theme through the library.

use oxikube_theme::tokens::{SyntaxStyle, SyntaxTheme};

/// One capture name the editor paints, and the Zed name that stands in for it when the theme has
/// neither it nor an ancestor.
pub(super) struct Capture {
    /// The capture name, as the editor's highlighter lists it.
    pub(super) name: &'static str,
    /// A theme name to use instead; `None` when the capture falls back to plain text.
    pub(super) alias: Option<&'static str>,
}

const fn capture(name: &'static str, alias: Option<&'static str>) -> Capture {
    Capture { name, alias }
}

/// Every capture name the editor paints, in the highlighter's order.
pub(super) const EDITOR_CAPTURES: [Capture; 41] = [
    capture("attribute", None),
    capture("boolean", Some("constant")),
    capture("comment", None),
    capture("comment.doc", None),
    capture("constant", None),
    capture("constructor", Some("type")),
    capture("embedded", None),
    capture("emphasis", None),
    capture("emphasis.strong", None),
    capture("enum", Some("type")),
    capture("function", None),
    capture("hint", None),
    capture("keyword", None),
    capture("label", Some("keyword")),
    capture("link_text", Some("string")),
    capture("link_uri", Some("string")),
    capture("number", Some("constant")),
    capture("operator", None),
    capture("predictive", None),
    capture("preproc", Some("keyword")),
    capture("primary", None),
    capture("property", Some("variable")),
    capture("punctuation", None),
    capture("punctuation.bracket", None),
    capture("punctuation.delimiter", None),
    capture("punctuation.list_marker", None),
    capture("punctuation.special", None),
    capture("string", None),
    capture("string.escape", None),
    capture("string.regex", None),
    capture("string.special", None),
    capture("string.special.symbol", None),
    capture("tag", None),
    capture("tag.doctype", None),
    capture("text.code.span", Some("text.literal")),
    capture("text.literal", Some("string")),
    capture("title", Some("keyword")),
    capture("type", None),
    capture("variable", None),
    capture("variable.special", None),
    capture("variant", Some("enum")),
];

/// The editor capture a raw query capture is painted as: itself, or its longest dotted prefix in
/// [`EDITOR_CAPTURES`]. `None` for a capture the editor does not paint (plain text). The
/// library applies this rule itself; it is spelled out here so the tests pin it for every capture
/// the YAML and JSON queries emit.
#[cfg(test)]
pub(super) fn editor_capture(raw: &str) -> Option<&'static str> {
    let mut name = raw;
    loop {
        if let Some(found) = EDITOR_CAPTURES.iter().find(|c| c.name == name) {
            return Some(found.name);
        }
        name = name.rsplit_once('.')?.0;
    }
}

/// The style `theme` gives the editor capture `capture`: its own entry, its ancestors', then the
/// alias's. `None` paints plain text.
pub(super) fn resolve<'a>(theme: &'a SyntaxTheme, capture: &Capture) -> Option<&'a SyntaxStyle> {
    theme
        .style(capture.name)
        .or_else(|| capture.alias.and_then(|alias| theme.style(alias)))
}

#[cfg(test)]
mod tests;
