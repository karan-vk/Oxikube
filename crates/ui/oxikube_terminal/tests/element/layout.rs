//! Bounds -> cols x rows, and one resize per change.

use gpui::{TestAppContext, px, size};
use oxikube_ports::TerminalSize;

use super::{CELL, FONT_SIZE, ROW, harness};

#[gpui::test]
fn the_grid_fills_the_bounds_and_resizes_once(cx: &mut TestAppContext) {
    let mut h = harness(cx, 80. * CELL, 10. * ROW + 5., FONT_SIZE);
    assert_eq!(h.grid(), (80, 10));
    let resizes = h.backend.resizes();
    assert_eq!(
        resizes.last(),
        Some(&TerminalSize::new(80, 10).with_pixels(480, 130))
    );
    h.frame();
    h.frame();
    assert_eq!(
        h.backend.resizes(),
        resizes,
        "redraws without a size change send no resize"
    );

    h.window.simulate_resize(size(px(40. * CELL), px(5. * ROW)));
    h.frame();
    h.frame();
    assert_eq!(h.grid(), (40, 5));
    assert_eq!(
        h.backend.resizes().len(),
        resizes.len() + 1,
        "one resize per change"
    );
}

#[gpui::test]
fn the_font_size_sets_the_cell_size(cx: &mut TestAppContext) {
    for (font, expected) in [(10., (80, 20)), (20., (40, 10)), (8., (100, 25))] {
        let mut h = harness(cx, 480., 260., font);
        assert_eq!(h.grid(), expected, "font {font}");
    }
}
