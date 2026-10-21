use super::{MAX_SOURCE_BYTES, TRUNCATED_NOTE, is_safe_link, sanitize};

#[test]
fn plain_markdown_is_left_alone() {
    let md = "# Title\n\n- a\n  - nested\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n[link](https://x.io) and `code <b>`\n";
    assert_eq!(sanitize(md), md);
}

#[test]
fn fenced_code_keeps_its_html() {
    let md = "```html\n<script>alert(1)</script>\n<div>x</div>\n```\n\n~~~\n<b>\n~~~\n";
    assert_eq!(sanitize(md), md);
}

#[test]
fn tags_are_stripped_and_their_text_stays() {
    assert_eq!(
        sanitize("<p align=\"center\">Hello <b>world</b></p>\n"),
        "Hello world\n"
    );
    assert_eq!(sanitize("a<br/>b <img src=x onerror=y> c\n"), "ab  c\n");
}

#[test]
fn scripts_styles_and_comments_are_removed_with_their_content() {
    assert_eq!(sanitize("a<script>alert(1)</script>b\n"), "ab\n");
    assert_eq!(
        sanitize("a\n<script>\nevil()\n</script>\nb\n"),
        "a\n\n\n\nb\n"
    );
    assert_eq!(sanitize("x <style>p{}</style> y\n"), "x  y\n");
    assert_eq!(sanitize("a<!-- hidden -->b\n"), "ab\n");
    assert_eq!(sanitize("a\n<!--\nhidden\n-->\nb\n"), "a\n\n\n\nb\n");
}

#[test]
fn a_tag_spanning_lines_is_removed() {
    assert_eq!(sanitize("<a\n href=\"x\"\n>text</a>\n"), "\n\ntext\n");
}

#[test]
fn autolinks_and_comparisons_are_not_tags() {
    let md = "<https://example.com> and <me@x.io> and 1 < 2 and a<3\n";
    assert_eq!(sanitize(md), md);
}

#[test]
fn images_become_links() {
    assert_eq!(
        sanitize("![logo](https://x.io/a.png \"t\") ![](https://x.io/b.png)\n"),
        "[logo](https://x.io/a.png) [image](https://x.io/b.png)\n"
    );
}

#[test]
fn badges_become_a_single_link() {
    assert_eq!(
        sanitize("[![Build](https://ci/badge.svg)](https://ci/run)\n"),
        "[Build](https://ci/run)\n"
    );
}

#[test]
fn inline_code_keeps_brackets_and_images() {
    let md = "use `![a](b)` and `<x>` here\n";
    assert_eq!(sanitize(md), md);
}

#[test]
fn a_huge_document_is_cut_at_a_line_and_noted() {
    let line = "0123456789abcdef\n";
    let md = line.repeat(MAX_SOURCE_BYTES / line.len() + 100);
    let out = sanitize(&md);
    assert!(out.ends_with(TRUNCATED_NOTE));
    assert!(out.len() <= MAX_SOURCE_BYTES + TRUNCATED_NOTE.len());
    assert!(out.starts_with(line));
}

#[test]
fn only_web_and_mail_links_may_be_opened() {
    for ok in ["https://x.io", "HTTP://x.io", "mailto:a@b.c"] {
        assert!(is_safe_link(ok), "{ok}");
    }
    for bad in [
        "javascript:alert(1)",
        "file:///etc/passwd",
        "./docs",
        "#x",
        "",
        "ftp://x",
        "data:text/html,x",
    ] {
        assert!(!is_safe_link(bad), "{bad}");
    }
}
