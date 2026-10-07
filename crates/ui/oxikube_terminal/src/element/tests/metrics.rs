//! Cell metrics: pixels to cells and back.

use gpui::{point, px, size};
use oxikube_ports::TerminalSize;

use super::super::metrics::CellMetrics;
use crate::grid::SelectionSide;

fn metrics(font_size: f32, cell_width: f32) -> CellMetrics {
    CellMetrics::from_parts(
        px(font_size),
        px(cell_width),
        1.3,
        px(font_size * 0.8),
        px(font_size * 0.2),
    )
}

#[test]
fn rows_are_whole_pixels_and_hold_the_glyph_box() {
    let m = metrics(13., 7.8);
    assert_eq!(m.line_height, px(17.));
    assert!(m.baseline > px(0.) && m.baseline < m.line_height);
    let tall = CellMetrics::from_parts(px(10.), px(6.), 1.0, px(9.), px(4.));
    assert_eq!(
        tall.line_height,
        px(13.),
        "never shorter than ascent + descent"
    );
}

#[test]
fn bounds_give_whole_cells_for_several_font_sizes() {
    for (font, cell, bounds, expected) in [
        (13., 7.8, (800., 600.), (102, 35)),
        (16., 9.6, (800., 600.), (83, 28)),
        (10., 6.0, (800., 600.), (133, 46)),
        (13., 7.8, (640., 384.), (82, 22)),
    ] {
        let m = metrics(font, cell);
        let grid = m.grid_size(size(px(bounds.0), px(bounds.1)));
        assert_eq!((grid.width, grid.height), expected, "font {font}");
        assert_eq!(
            grid.pixel_width,
            (f32::from(grid.width) * cell).round() as u16
        );
    }
}

#[test]
fn tiny_bounds_still_give_the_minimum_grid() {
    let m = metrics(13., 7.8);
    let grid = m.grid_size(size(px(3.), px(2.)));
    assert_eq!((grid.width, grid.height), (2, 1));
    assert_eq!(
        TerminalSize::new(grid.width, grid.height),
        TerminalSize::new(2, 1)
    );
}

#[test]
fn positions_map_to_cells_and_halves() {
    let m = metrics(10., 6.);
    let origin = point(px(100.), px(50.));
    assert_eq!(
        m.cell_at(origin, point(px(100.), px(50.)), 80, 24),
        (0, 0, SelectionSide::Left)
    );
    assert_eq!(
        m.cell_at(
            origin,
            point(px(100. + 6. * 3. + 4.), px(50. + 13. * 2. + 1.)),
            80,
            24
        ),
        (2, 3, SelectionSide::Right)
    );
    // Outside the grid: clamped.
    assert_eq!(m.cell_at(origin, point(px(5000.), px(5000.)), 80, 24).0, 23);
    assert_eq!(m.cell_at(origin, point(px(0.), px(0.)), 80, 24).1, 0);
    assert_eq!(m.cell_origin(origin, 2, 3), point(px(118.), px(76.)));
}
