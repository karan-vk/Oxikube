//! The geometry cache: the layout and tessellated paths of a chart, reused every frame until the
//! data, the size, the zoom or the theme changes.
//!
//! This is what keeps a chart that stays on screen cheap: a repaint is a handful of quads and
//! translated path copies; projecting, decimating, tick choice, text measuring and tessellation
//! happen once per metrics tick or resize.

use super::layout::Layout;
use super::palette::ChartPalette;
use super::spec::Spec;
use crate::size::current_scale;
use gpui::{Bounds, Pixels, Point, Size, Window, point, px};
use gpui_component::plot::PathCache;

/// What a [`Layout`] was built from. Equal keys mean the same geometry.
#[derive(Clone, PartialEq)]
struct Key {
    generation: u64,
    width: u32,
    height: u32,
    scale: u32,
    palette: ChartPalette,
}

/// A chart's cached geometry.
pub(super) struct LayoutCache {
    key: Option<Key>,
    pub layout: Option<Layout>,
    /// Counts rebuilds; the key of the path caches.
    pub build: u64,
    pub strokes: Vec<PathCache>,
    pub fills: Vec<PathCache>,
    /// Where the canvas was last painted, in window coordinates (mouse events use it).
    pub origin: Point<Pixels>,
    /// The canvas size at the last paint.
    pub canvas_size: Size<Pixels>,
    /// How many times the layout was built (tests assert it stays put while repainting).
    pub builds: usize,
    /// The `build` the hover overlay was last positioned from; `None` while there is no overlay.
    pub overlay_build: Option<u64>,
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self {
            key: None,
            layout: None,
            build: 0,
            strokes: Vec::new(),
            fills: Vec::new(),
            origin: point(px(0.), px(0.)),
            canvas_size: Size::default(),
            builds: 0,
            overlay_build: None,
        }
    }
}

impl LayoutCache {
    /// Whether the hover overlay was positioned from an older layout than the current one.
    pub fn overlay_is_stale(&self) -> bool {
        self.overlay_build.is_some_and(|b| b != self.build)
    }

    /// Rebuilds the layout when `generation` (the spec's), the canvas size, the zoom or the
    /// palette differ from the last build.
    pub fn ensure(
        &mut self,
        generation: u64,
        spec: &Spec,
        bounds: Bounds<Pixels>,
        palette: &ChartPalette,
        window: &mut Window,
    ) {
        let key = Key {
            generation,
            width: bounds.size.width.as_f32().to_bits(),
            height: bounds.size.height.as_f32().to_bits(),
            scale: current_scale().to_bits(),
            palette: *palette,
        };
        if self.key.as_ref() == Some(&key) && self.layout.is_some() {
            return;
        }
        let layout = Layout::build(spec, bounds.size, palette, window);
        self.strokes
            .resize_with(layout.series.len(), PathCache::default);
        self.fills
            .resize_with(layout.series.len(), PathCache::default);
        self.layout = Some(layout);
        self.key = Some(key);
        self.build += 1;
        self.builds += 1;
    }
}
