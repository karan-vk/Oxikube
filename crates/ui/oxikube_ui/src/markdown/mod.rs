//! Rendered Markdown (docs, release notes, chart READMEs, agent replies).
//!
//! The renderer is gpui-component's text view, re-exported under our names. Markdown that comes
//! from outside (a chart's README is written by the chart's author) goes through [`sanitize`]
//! first: raw HTML (tags, comments, `<script>`, `<style>`) is stripped, images become links so
//! nothing is fetched from the network just by showing a document, and a huge document is cut. It
//! is a pure function over text, so callers run it on a background thread. Links are opened only
//! when [`is_safe_link`] says so.
//!
//! The text view parses off the UI thread and virtualises long documents; give it a
//! [`MarkdownState`] (`cx.new(|cx| MarkdownState::markdown(&text, cx))`) and render it with
//! [`MarkdownView::new`].

mod sanitize;

#[cfg(test)]
mod tests;

pub use gpui_component::text::{
    TextView as MarkdownView, TextViewState as MarkdownState, TextViewStyle as MarkdownStyle,
    markdown,
};
pub use sanitize::{MAX_SOURCE_BYTES, TRUNCATED_NOTE, is_safe_link, sanitize};
