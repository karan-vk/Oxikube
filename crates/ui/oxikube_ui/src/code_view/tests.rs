//! The row map (pure) and the view in a test window: text laid out off the UI thread, only the
//! visible rows built, colours, selection and copy.

use std::sync::Arc;

use gpui::{Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px, size};

use super::rows::{MAX_ROW_COLS, MIN_WRAP_COLS, RowMap, char_cols, offset_at_col};
use super::{CodeView, Look};

fn texts(map: &RowMap, text: &str) -> Vec<String> {
    (0..map.len())
        .map(|ix| {
            let row = map.row(ix).unwrap();
            text[row.start..row.end].to_owned()
        })
        .collect()
}

#[test]
fn unwrapped_rows_are_the_lines() {
    let text = "a: 1\nb: two\r\n\nlast";
    let map = RowMap::build(text, None, false);
    assert_eq!(texts(&map, text), ["a: 1", "b: two", "", "last"]);
    assert_eq!(map.lines(), 4);
    assert_eq!(map.widest(), 1);
    assert_eq!(map.cols(), MAX_ROW_COLS);
    assert!((0..4).all(|ix| map.starts_line(ix)));
}

#[test]
fn a_trailing_newline_adds_no_row_and_an_empty_text_has_one() {
    assert_eq!(RowMap::build("x\n", None, false).len(), 1);
    let empty = RowMap::build("", None, false);
    assert_eq!(empty.len(), 1);
    assert_eq!(empty.row(0).unwrap().start, 0);
}

#[test]
fn wrapping_breaks_after_a_space_and_numbers_only_the_first_row() {
    // 20 columns wide, 3 of them for the gutter (one digit and a column either side).
    let text = "short\nthe quick brown fox jumps over\n";
    let map = RowMap::build(text, Some(MIN_WRAP_COLS + 3), true);
    assert_eq!(map.gutter_cols(), 3);
    assert_eq!(map.cols(), MIN_WRAP_COLS);
    assert_eq!(
        texts(&map, text),
        ["short", "the quick brown ", "fox jumps over"]
    );
    assert!(map.starts_line(1));
    assert!(!map.starts_line(2), "a continuation row has no number");
    assert_eq!(map.lines(), 2);
    assert_eq!(map.first_row_of_line(1), 1);
}

#[test]
fn a_word_longer_than_the_row_is_cut_where_it_overflows() {
    let word = "x".repeat(40);
    let map = RowMap::build(&word, Some(MIN_WRAP_COLS), false);
    assert_eq!(
        texts(&map, &word),
        [&word[..16], &word[16..32], &word[32..]]
    );
}

#[test]
fn an_unwrapped_line_longer_than_the_cap_continues_on_the_next_row() {
    let line = "y".repeat(MAX_ROW_COLS * 2 + 5);
    let map = RowMap::build(&line, None, false);
    assert_eq!(map.len(), 3);
    assert_eq!(map.lines(), 1);
    assert_eq!(map.row(2).unwrap().end - map.row(2).unwrap().start, 5);
}

#[test]
fn the_widest_row_of_a_cut_line_is_a_full_row_not_its_short_tail() {
    // A 2500-column line cut into 1000, 1000 and 500: the horizontal extent is measured from
    // `widest()`, so it must point at a 1000-column row or the full rows are clipped.
    let text = format!(
        "short\n{}\n",
        "z".repeat(MAX_ROW_COLS * 2 + MAX_ROW_COLS / 2)
    );
    let map = RowMap::build(&text, None, true);
    assert_eq!(map.len(), 4);
    let widest = map.row(map.widest()).unwrap();
    assert_eq!(widest.end - widest.start, MAX_ROW_COLS);

    // The same with word wrap: the widest row is the longest wrapped row, not the line's last.
    let text = "aaaaaaaaaaaaaaa bbbbbbbbbbbbbbb c";
    let map = RowMap::build(text, Some(MIN_WRAP_COLS), false);
    assert_eq!(
        texts(&map, text),
        ["aaaaaaaaaaaaaaa ", "bbbbbbbbbbbbbbb ", "c"]
    );
    assert_eq!(map.widest(), 0);
}

#[test]
fn wide_characters_take_two_columns_and_rows_end_on_char_boundaries() {
    assert_eq!(char_cols('a'), 1);
    assert_eq!(char_cols('漢'), 2);
    let text = "漢".repeat(20);
    let map = RowMap::build(&text, Some(MIN_WRAP_COLS), false);
    assert_eq!(map.len(), 3, "8 characters (16 columns) a row");
    for ix in 0..map.len() {
        let row = map.row(ix).unwrap();
        assert!(text.is_char_boundary(row.start) && text.is_char_boundary(row.end));
    }
    assert_eq!(
        offset_at_col("漢字x", 1),
        3,
        "the nearer edge of a wide character"
    );
    assert_eq!(offset_at_col("漢字x", 4), 6);
    assert_eq!(offset_at_col("abc", 99), 3);
}

#[test]
fn a_five_megabyte_text_is_mapped_with_one_row_per_short_line() {
    let line = "service.0.route.1: upstream=payments-1.svc.cluster.local:8080 timeout=30s\n";
    let text = line.repeat(5 * 1024 * 1024 / line.len());
    let map = RowMap::build(&text, Some(120), true);
    assert_eq!(map.len(), map.lines());
    assert_eq!(map.lines(), text.lines().count());
}

#[test]
fn the_style_cache_holds_the_measured_row_and_the_visible_rows_together() {
    use super::highlight::{Parsed, StyleCache, row_styles};
    use gpui_component::highlighter::HighlightTheme;

    let text = config_map_yaml(64 * 1024);
    let parsed = Parsed::parse(&text, "yaml");
    let theme = HighlightTheme::default_dark();
    let mut cache = StyleCache::default();
    let top = 0..200;
    let below = 40_000..40_400;
    for _ in 0..5 {
        // What a frame asks for when scrolled: the first row (measured), then the visible rows.
        cache.styles(&parsed, &(0..20), top.clone(), &theme);
        cache.styles(&parsed, &(40_100..40_200), below.clone(), &theme);
    }
    assert_eq!(cache.reads, 2, "each range is read from the tree once");
    let styles = cache.styles(&parsed, &(0..20), top, &theme).to_vec();
    assert!(
        styles.iter().any(|(_, style)| style.color.is_some()),
        "the YAML keys are coloured"
    );
    let row = row_styles(&styles, 15..30);
    assert!(
        row.iter().all(|(range, _)| range.end <= 15),
        "relative to the row"
    );
}

// --- The view in a window ---------------------------------------------------------------------

/// A YAML text of about `bytes` bytes shaped like a big ConfigMap: blocks of 52 lines.
fn config_map_yaml(bytes: usize) -> String {
    let mut text = String::from("apiVersion: v1\nkind: ConfigMap\ndata:\n");
    let mut k = 0;
    while text.len() < bytes {
        text.push_str(&format!("  routes-{k:04}.properties: |\n"));
        for line in 0..52 {
            text.push_str(&format!(
                "    service.{k}.route.{line}: upstream=payments-{line}.svc.cluster.local:8080 \
                 timeout=30s retries=3\n"
            ));
        }
        k += 1;
    }
    text
}

fn mount(cx: &mut TestAppContext, look: Look) -> (Entity<CodeView>, &mut VisualTestContext) {
    cx.update(crate::init);
    let (view, vcx) = cx.add_window_view(move |_, cx| CodeView::new(look, cx));
    vcx.simulate_resize(size(px(900.), px(600.)));
    (view, vcx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// Draws, lets the background work land, and draws again.
fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

fn set_text(view: &Entity<CodeView>, text: &Arc<str>, cx: &mut VisualTestContext) {
    let text = text.clone();
    cx.update(|_, cx| view.update(cx, |view, cx| view.set_text(text, cx)));
}

#[gpui::test]
fn a_megabyte_text_is_laid_out_off_the_ui_thread_and_only_visible_rows_are_built(
    cx: &mut TestAppContext,
) {
    let (view, cx) = mount(cx, Look::YAML);
    draw(cx);
    let text: Arc<str> = config_map_yaml(1024 * 1024).into();
    set_text(&view, &text, cx);
    assert!(
        !view.read_with(cx, |v, _| v.is_ready()),
        "set_text lays nothing out on the UI thread"
    );
    settle(cx);
    let (ready, rows, drawn, same, coloured) = view.read_with(cx, |v, _| {
        (
            v.is_ready(),
            v.row_count(),
            v.rows_drawn(),
            v.text().is_some_and(|t| Arc::ptr_eq(t, &text)),
            v.is_highlighted(),
        )
    });
    assert!(ready);
    assert!(same, "the view holds the text given, not a copy");
    assert!(rows >= text.lines().count(), "{rows} rows");
    assert!(
        (1..=40).contains(&drawn),
        "a 600 px view builds its visible rows only, not {drawn} of {rows}"
    );
    assert!(
        coloured,
        "the YAML is parsed off the UI thread and coloured"
    );
    assert_eq!(view.read_with(cx, |v, _| v.language()), Some("yaml"));
}

#[gpui::test]
fn a_new_text_replaces_the_shown_one_only_when_laid_out_and_coloured(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx, Look::YAML);
    let first: Arc<str> = "kind: Pod\nmetadata:\n  name: web-0\n".into();
    set_text(&view, &first, cx);
    settle(cx);
    assert!(view.read_with(cx, |v, _| v.is_highlighted()));

    let second: Arc<str> = "kind: Pod\nmetadata:\n  name: web-1\n".into();
    set_text(&view, &second, cx);
    let (shown, current) = view.read_with(cx, |v, _| (v.text().cloned(), v.is_current()));
    assert!(
        shown.is_some_and(|t| Arc::ptr_eq(&t, &first)),
        "the old text stays meanwhile"
    );
    assert!(!current);
    cx.run_until_parked();
    let (shown, coloured) = view.read_with(cx, |v, _| (v.text().cloned(), v.is_highlighted()));
    assert!(shown.is_some_and(|t| Arc::ptr_eq(&t, &second)));
    assert!(
        coloured,
        "the new text arrives with its colours: no uncoloured frame"
    );
}

/// The line at the top of the view.
fn top_line(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> Option<usize> {
    view.read_with(cx, |v, _| {
        v.shown
            .as_ref()
            .and_then(|shown| shown.rows.row(v.top_row()))
            .map(|row| row.line)
    })
}

#[gpui::test]
fn a_narrower_view_rewraps_off_the_ui_thread_and_keeps_the_top_line(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx, Look::YAML);
    let long = format!("key: {}\n", "word ".repeat(200));
    let text: Arc<str> = long.repeat(50).into();
    set_text(&view, &text, cx);
    settle(cx);
    let (wide_cols, wide_rows) = view.read_with(cx, |v, _| (v.wrap_cols(), v.row_count()));
    // Scroll to the first row of line 10.
    cx.update(|_, cx| {
        view.update(cx, |v, _| {
            let row = v.shown.as_ref().unwrap().rows.first_row_of_line(10);
            v.set_scroll_y(-(v.metrics.row_height * row as f32));
        })
    });
    draw(cx);
    assert_eq!(top_line(&view, cx), Some(10));

    cx.simulate_resize(size(px(450.), px(600.)));
    draw(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.row_count()),
        wide_rows,
        "the old rows stay until the new ones are ready"
    );
    // A second width while that layout runs: picked up when it lands, not lost.
    cx.simulate_resize(size(px(500.), px(600.)));
    draw(cx);
    cx.run_until_parked();
    draw(cx);
    let (narrow_cols, narrow_rows) = view.read_with(cx, |v, _| (v.wrap_cols(), v.row_count()));
    assert!(narrow_cols < wide_cols, "{narrow_cols:?} < {wide_cols:?}");
    assert!(narrow_rows > wide_rows, "{narrow_rows} > {wide_rows}");
    let expected = view
        .read_with(cx, |v, _| v.metrics.width_cols)
        .map(|width| RowMap::build(&text, Some(width), true).cols());
    assert_eq!(narrow_cols, expected, "laid out for the last width");
    assert_eq!(
        top_line(&view, cx),
        Some(10),
        "the same line stays at the top"
    );
}

#[gpui::test]
fn plain_text_scrolls_sideways_and_a_huge_line_is_cut_into_bounded_rows(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx, Look::TEXT);
    let text: Arc<str> = format!("Name: web\n{}\n", "x".repeat(MAX_ROW_COLS * 3)).into();
    set_text(&view, &text, cx);
    settle(cx);
    let (rows, wrap, coloured) = view.read_with(cx, |v, _| {
        (v.row_count(), v.wrap_cols(), v.is_highlighted())
    });
    assert_eq!(rows, 4, "one row, then the long line in three bounded runs");
    assert_eq!(wrap, None, "no soft wrap");
    assert!(!coloured, "no grammar");
}

#[gpui::test]
fn dragging_selects_and_the_copy_keys_copy_it(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx, Look::YAML);
    let text: Arc<str> = "kind: Pod\nmetadata:\n  name: web-0\n".into();
    set_text(&view, &text, cx);
    settle(cx);
    let bounds = cx.debug_bounds("code-view").expect("the view is drawn");
    let (advance, padding, gutter) = view.read_with(cx, |v, _| {
        let gutter = v.shown.as_ref().map_or(0, |s| s.rows.gutter_cols());
        (v.metrics.advance, v.metrics.padding, gutter)
    });
    let left = bounds.left() + advance * gutter as f32 + padding;
    let y = bounds.top() + px(10.);
    let none = Modifiers::none();
    cx.simulate_mouse_down(point(left, y), MouseButton::Left, none);
    cx.simulate_mouse_move(point(left + advance * 4., y), MouseButton::Left, none);
    cx.simulate_mouse_up(point(left + advance * 4., y), MouseButton::Left, none);
    assert_eq!(
        view.read_with(cx, |v, _| v.selected_text().map(str::to_owned)),
        Some("kind".to_owned())
    );
    let (copy, all) = if cfg!(target_os = "macos") {
        ("cmd-c", "cmd-a")
    } else {
        ("ctrl-c", "ctrl-a")
    };
    cx.simulate_keystrokes(copy);
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("kind")
    );

    // Select all, then copy: the whole text.
    cx.simulate_keystrokes(all);
    cx.simulate_keystrokes(copy);
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some(&*text)
    );
}

#[test]
fn the_row_of_a_byte_follows_the_wrapped_rows() {
    let text = "short\nthe quick brown fox jumps over\n";
    let map = RowMap::build(text, Some(MIN_WRAP_COLS + 3), true);
    assert_eq!(map.row_of_byte(0), 0);
    assert_eq!(
        map.row_of_byte(5),
        0,
        "the newline belongs to the row it ends"
    );
    assert_eq!(map.row_of_byte(6), 1);
    assert_eq!(map.row_of_byte(22), 2, "the wrapped continuation");
    assert_eq!(map.row_of_byte(10_000), 2, "past the end: the last row");
}

#[gpui::test]
fn matches_are_drawn_over_their_text_and_scrolled_to(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx, Look::YAML);
    draw(cx);
    let text: Arc<str> = config_map_yaml(200 * 1024).into();
    set_text(&view, &text, cx);
    settle(cx);

    // The last "route.51" of the first block, deep below the first screen.
    let needle = "service.1.route.51";
    let at = text.find(needle).expect("in the text");
    let ranges: Arc<[std::ops::Range<usize>]> = vec![at..at + needle.len()].into();
    view.update(cx, |view, cx| {
        view.set_matches(text.clone(), ranges, Some(0), cx);
        view.scroll_to_byte(at, cx);
    });
    settle(cx);
    let (top, visible, rows) = view.read_with(cx, |v, _| {
        (v.top_visible_row(), v.visible_rows(), v.row_count())
    });
    let row = text[..at].matches('\n').count();
    assert!(visible > 0 && rows > visible);
    assert!(
        (top..top + visible).contains(&row),
        "the match is on screen: row {row}, screen {top}..{}",
        top + visible
    );
    assert!(top > 0, "it was below the first screen");

    // A match already in view does not move the screen.
    view.update(cx, |view, cx| view.scroll_to_byte(at, cx));
    settle(cx);
    assert_eq!(view.read_with(cx, |v, _| v.top_visible_row()), top);

    // Cleared matches draw nothing and a newer text ignores the ranges of the older one.
    view.update(cx, |view, cx| view.clear_matches(cx));
    settle(cx);
}

#[gpui::test]
fn ranges_found_in_an_older_text_are_not_drawn_on_a_newer_one(cx: &mut TestAppContext) {
    let (view, cx) = mount(cx, Look::TEXT);
    let first: Arc<str> = "alpha beta\n".into();
    set_text(&view, &first, cx);
    settle(cx);
    let ranges: Arc<[std::ops::Range<usize>]> = vec![0..5].into();
    view.update(cx, |view, cx| {
        view.set_matches(first.clone(), ranges, None, cx);
    });
    let second: Arc<str> = "gamma delta\n".into();
    set_text(&view, &second, cx);
    // Must not panic or slice a char boundary of the new text: the ranges are for the old one.
    settle(cx);
    assert!(view.read_with(cx, |v, _| v.is_current()));
}
