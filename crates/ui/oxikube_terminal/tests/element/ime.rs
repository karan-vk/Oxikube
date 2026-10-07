//! IME composition through the input handler API: marked text, commit, the candidate window.

use gpui::{Bounds, EntityInputHandler as _, TestAppContext, point, px, size};

use super::{CELL, FONT_SIZE, Harness, ROW, harness};

/// Calls the terminal's input handler like the platform does.
fn ime<R>(
    h: &mut Harness,
    f: impl FnOnce(
        &mut oxikube_terminal::TerminalState,
        &mut gpui::Window,
        &mut gpui::Context<oxikube_terminal::TerminalState>,
    ) -> R,
) -> R {
    let terminal = h.terminal.clone();
    h.window
        .update(|window, cx| terminal.update(cx, |terminal, cx| f(terminal, window, cx)))
}

fn mark(h: &mut Harness, text: &str, selected: Option<std::ops::Range<usize>>) {
    ime(h, |t, w, cx| {
        t.replace_and_mark_text_in_range(None, text, selected, w, cx)
    });
}

fn composition(h: &mut Harness) -> Option<String> {
    h.terminal.read_with(&mut *h.window, |t, _| {
        t.composition().map(|composition| composition.text.clone())
    })
}

#[gpui::test]
fn composition_is_marked_not_sent_and_committed_as_utf8(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    mark(&mut h, "に", Some(1..1));
    h.frame();
    assert_eq!(composition(&mut h).as_deref(), Some("に"));
    assert_eq!(
        ime(&mut h, |t, w, cx| t.marked_text_range(w, cx)),
        Some(0..1)
    );
    assert_eq!(
        ime(&mut h, |t, w, cx| t.selected_text_range(false, w, cx)).map(|s| s.range),
        Some(1..1)
    );
    assert_eq!(h.written(), b"", "marked text never reaches the process");

    mark(&mut h, "にほ", None);
    h.frame();
    assert_eq!(composition(&mut h).as_deref(), Some("にほ"));
    assert_eq!(
        ime(&mut h, |t, w, cx| t.marked_text_range(w, cx)),
        Some(0..2)
    );
    assert_eq!(h.written(), b"");

    ime(&mut h, |t, w, cx| {
        t.replace_text_in_range(None, "日本", w, cx)
    });
    h.frame();
    assert_eq!(composition(&mut h), None, "the commit ends the composition");
    assert_eq!(h.written(), "日本".as_bytes(), "committed text is UTF-8");
    assert_eq!(ime(&mut h, |t, w, cx| t.marked_text_range(w, cx)), None);
}

#[gpui::test]
fn text_for_range_reads_the_composition_in_utf16_units(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    assert_eq!(
        ime(&mut h, |t, w, cx| t.text_for_range(0..1, &mut None, w, cx)),
        None,
        "nothing composing"
    );
    // "a" is one UTF-16 unit, the emoji two.
    mark(&mut h, "a😀b", None);
    let mut adjusted = None;
    let text = ime(&mut h, |t, w, cx| {
        t.text_for_range(1..3, &mut adjusted, w, cx)
    });
    assert_eq!(text.as_deref(), Some("😀"));
    assert_eq!(adjusted, Some(1..3));
    let mut adjusted = None;
    let text = ime(&mut h, |t, w, cx| {
        t.text_for_range(0..99, &mut adjusted, w, cx)
    });
    assert_eq!(text.as_deref(), Some("a😀b"));
    assert_eq!(adjusted, Some(0..4), "clamped to the composition");
}

#[gpui::test]
fn emptying_or_unmarking_ends_the_composition_without_sending(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    mark(&mut h, "か", None);
    mark(&mut h, "", None);
    assert_eq!(composition(&mut h), None);
    mark(&mut h, "き", None);
    ime(&mut h, |t, w, cx| t.unmark_text(w, cx));
    assert_eq!(composition(&mut h), None);
    assert_eq!(h.written(), b"");
}

#[gpui::test]
fn the_candidate_window_sits_at_the_cursor(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("ab\r\ncd>");
    h.output("");
    mark(&mut h, "にほ", None);
    h.frame();
    let element = Bounds::new(point(px(10.), px(20.)), size(px(480.), px(130.)));
    // The cursor is on row 1, column 3.
    let whole = ime(&mut h, |t, w, cx| t.bounds_for_range(0..2, element, w, cx)).unwrap();
    assert_eq!(whole.origin, point(px(10. + 3. * CELL), px(20. + ROW)));
    assert_eq!(
        whole.size,
        size(px(4. * CELL), px(ROW)),
        "two wide glyphs: four cells"
    );
    // The second glyph starts two cells further.
    let second = ime(&mut h, |t, w, cx| t.bounds_for_range(1..2, element, w, cx)).unwrap();
    assert_eq!(second.origin.x, px(10. + 5. * CELL));
    assert_eq!(second.size.width, px(2. * CELL));
}

#[gpui::test]
fn keys_belong_to_the_input_method_while_it_composes(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    mark(&mut h, "に", None);
    h.window.simulate_keystrokes("up backspace");
    assert_eq!(h.written(), b"", "the IME consumes them");
    ime(&mut h, |t, w, cx| {
        t.replace_text_in_range(None, "に", w, cx)
    });
    h.window.simulate_keystrokes("enter");
    assert_eq!(
        h.written(),
        "に\r".as_bytes(),
        "keys are the terminal's again"
    );
}

#[gpui::test]
fn the_composition_is_painted_without_disturbing_the_grid(cx: &mut TestAppContext) {
    let mut h = harness(cx, 480., 130., FONT_SIZE);
    h.output("$ ");
    let before = h.state.cache_stats();
    mark(&mut h, "にほんご", None);
    h.frame();
    h.frame();
    let after = h.state.cache_stats();
    assert_eq!(
        after.shaped_runs, before.shaped_runs,
        "the grid's rows are untouched: the preedit is drawn over them"
    );
    let row = h
        .terminal
        .read_with(&mut *h.window, |t, _| t.snapshot().row_text(0));
    assert_eq!(row, "$");
}
