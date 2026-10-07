//! Link detection on the hovered line: OSC 8, URLs, paths.

use std::path::Path;

use oxikube_ports::TerminalSize;

use super::super::links::{LinkKind, TerminalLink, link_at};
use crate::grid::{GridEvent, TermGrid, TerminalSnapshot};

fn snapshot(columns: u16, rows: u16, bytes: &str) -> TerminalSnapshot {
    let mut grid = TermGrid::new(TerminalSize::new(columns, rows), 0);
    let mut events: Vec<GridEvent> = Vec::new();
    grid.advance(bytes.as_bytes(), &mut events);
    grid.snapshot()
}

fn target(link: Option<TerminalLink>) -> Option<(String, LinkKind)> {
    link.map(|link| (link.target, link.kind))
}

#[test]
fn osc8_links_win_and_cover_their_cells() {
    let snap = snapshot(
        30,
        2,
        "see \x1b]8;;https://example.com/x\x1b\\the docs\x1b]8;;\x1b\\ now",
    );
    let link = link_at(&snap, 0, 6, false).expect("a link");
    assert_eq!(link.target, "https://example.com/x");
    assert_eq!(link.kind, LinkKind::Url);
    assert_eq!(link.cells, [(0, 4, 11)]);
    assert!(link.covers(0, 11) && !link.covers(0, 12));
    assert_eq!(link_at(&snap, 0, 13, false), None);
}

#[test]
fn urls_in_the_text_are_found_without_trailing_punctuation() {
    let snap = snapshot(
        60,
        2,
        "docs: https://kubernetes.io/docs/home. (see http://a.b/c_(d))",
    );
    assert_eq!(
        target(link_at(&snap, 0, 10, false)),
        Some(("https://kubernetes.io/docs/home".into(), LinkKind::Url))
    );
    assert_eq!(
        target(link_at(&snap, 0, 47, false)),
        Some(("http://a.b/c_(d)".into(), LinkKind::Url))
    );
    assert_eq!(link_at(&snap, 0, 2, true), None, "`docs:` is not a link");
}

#[test]
fn a_url_wrapped_over_two_rows_is_one_link() {
    let snap = snapshot(20, 3, "go https://example.com/a/long/path ok");
    let link = link_at(&snap, 1, 2, false).expect("the wrapped part");
    assert_eq!(link.target, "https://example.com/a/long/path");
    assert_eq!(link.cells, [(0, 3, 19), (1, 0, 13)]);
}

#[test]
fn paths_with_positions_are_links_when_paths_are_on() {
    let snap = snapshot(
        60,
        3,
        "error --> src/main.rs:12:3\r\nsee /etc/hosts and ~/notes.md\r\nmain.rs:7 v1.2 12:30",
    );
    assert_eq!(
        target(link_at(&snap, 0, 12, true)),
        Some(("src/main.rs:12:3".into(), LinkKind::Path))
    );
    assert_eq!(link_at(&snap, 0, 12, false), None, "paths off");
    assert_eq!(
        target(link_at(&snap, 1, 6, true)),
        Some(("/etc/hosts".into(), LinkKind::Path))
    );
    assert_eq!(
        target(link_at(&snap, 1, 22, true)),
        Some(("~/notes.md".into(), LinkKind::Path))
    );
    assert_eq!(
        target(link_at(&snap, 2, 2, true)),
        Some(("main.rs:7".into(), LinkKind::Path))
    );
    assert_eq!(link_at(&snap, 2, 11, true), None, "a version is no path");
    assert_eq!(link_at(&snap, 2, 16, true), None, "a time is no path");
    assert_eq!(link_at(&snap, 0, 1, true), None, "a plain word is no path");
}

#[test]
fn relative_paths_resolve_against_the_base() {
    let link = TerminalLink {
        target: "src/main.rs:12:3".into(),
        kind: LinkKind::Path,
        cells: Vec::new(),
    };
    assert_eq!(
        link.resolve(Some(Path::new("/work/app"))).as_deref(),
        Some("/work/app/src/main.rs:12:3")
    );
    assert_eq!(link.resolve(None), None, "no base, no target");
    let absolute = TerminalLink {
        target: "/etc/hosts".into(),
        ..link
    };
    assert_eq!(absolute.resolve(None).as_deref(), Some("/etc/hosts"));
    let url = TerminalLink {
        target: "https://x.y".into(),
        kind: LinkKind::Url,
        cells: Vec::new(),
    };
    assert_eq!(url.resolve(None).as_deref(), Some("https://x.y"));
}
