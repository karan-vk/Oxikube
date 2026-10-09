//! The text system GPUI holds: the platform's, with prepared glyphs answered from the warmer.
//!
//! Every method delegates to the platform text system. Only two differ: a glyph's raster bounds
//! and its bitmap come from the warmer when it prepared that exact glyph (same font, glyph, size,
//! subpixel variant, scale and dilation), which it did with the same platform calls. GPUI's other
//! calls only note that the text system is in use, so the worker keeps out of the frame.

use super::warmer::Shared;
use gpui::{
    Bounds, DevicePixels, Font, FontId, FontMetrics, FontRun, GlyphId, Hsla, LineLayout,
    MissingGlyphSink, Pixels, PlatformTextSystem, RenderGlyphParams, Result, Size,
    TextRenderingMode,
};
use std::borrow::Cow;
use std::sync::Arc;

impl PlatformTextSystem for Shared {
    fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> Result<()> {
        self.touch();
        self.inner.add_fonts(fonts)
    }

    fn set_missing_glyph_sink(&self, sink: Option<Arc<dyn MissingGlyphSink>>) {
        self.inner.set_missing_glyph_sink(sink);
    }

    fn all_font_names(&self) -> Vec<String> {
        self.inner.all_font_names()
    }

    fn font_id(&self, descriptor: &Font) -> Result<FontId> {
        self.touch();
        self.inner.font_id(descriptor)
    }

    fn prewarm_fonts(&self, font_ids: &[FontId]) {
        self.touch();
        self.inner.prewarm_fonts(font_ids);
    }

    fn font_metrics(&self, font_id: FontId) -> FontMetrics {
        self.inner.font_metrics(font_id)
    }

    fn typographic_bounds(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Bounds<f32>> {
        self.inner.typographic_bounds(font_id, glyph_id)
    }

    fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> Result<Size<f32>> {
        self.inner.advance(font_id, glyph_id)
    }

    fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
        self.inner.glyph_for_char(font_id, ch)
    }

    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        self.touch();
        match self.prepared_bounds(params) {
            Some(bounds) => Ok(bounds),
            None => self.inner.glyph_raster_bounds(params),
        }
    }

    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        raster_bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        self.touch();
        if let Some(prepared) = self.take_prepared(params, raster_bounds) {
            return Ok(prepared);
        }
        let raster = self.inner.rasterize_glyph(params, raster_bounds);
        self.touch();
        if raster.is_ok() {
            self.rasterised(params);
        }
        raster
    }

    fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
        self.touch();
        self.inner.layout_line(text, font_size, runs)
    }

    fn recommended_rendering_mode(&self, font_id: FontId, font_size: Pixels) -> TextRenderingMode {
        self.inner.recommended_rendering_mode(font_id, font_size)
    }

    fn glyph_dilation_for_color(&self, color: Hsla) -> u8 {
        self.inner.glyph_dilation_for_color(color)
    }
}
