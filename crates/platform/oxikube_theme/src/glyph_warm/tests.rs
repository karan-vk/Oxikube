//! The warmer against a scripted platform text system, its plan from the bundled themes, and the
//! plan following the active theme in a GPUI app.

use super::*;
use crate::appearance::{Appearance, SystemAppearance};
use crate::tokens::ThemeTokens;
use gpui::{
    Bounds, DevicePixels, Font, FontId, FontMetrics, FontRun, GlyphId, Hsla, LineLayout, Pixels,
    PlatformTextSystem, RenderGlyphParams, Result, Rgba, Size, TestAppContext, TextRenderingMode,
    UpdateGlobal as _, point, px, size,
};
use oxikube_settings::SettingsStore;
use parking_lot::Mutex;
use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// A platform text system that records its calls. A glyph's bitmap depends on every parameter
/// (so a wrong one shows); glyph 0 is empty, glyph 1 000 000 is bigger than the budget.
#[derive(Default)]
struct Fake {
    rasterised: Mutex<Vec<RenderGlyphParams>>,
    bounds_calls: AtomicUsize,
}

const EMPTY: u32 = 0;
const HUGE: u32 = 1_000_000;

fn bounds_of(glyph: u32) -> Bounds<DevicePixels> {
    let side = DevicePixels(if glyph == EMPTY { 0 } else { 4 });
    Bounds::new(point(DevicePixels(0), DevicePixels(-3)), size(side, side))
}

fn bitmap_of(params: &RenderGlyphParams) -> Vec<u8> {
    let len = if params.glyph_id.0 == HUGE {
        PREPARED_BUDGET + 1
    } else {
        16
    };
    let seed = params.glyph_id.0 as u8 ^ params.dilation << 4 ^ params.subpixel_variant.x;
    vec![seed; len]
}

impl PlatformTextSystem for Fake {
    fn add_fonts(&self, _: Vec<Cow<'static, [u8]>>) -> Result<()> {
        Ok(())
    }
    fn all_font_names(&self) -> Vec<String> {
        Vec::new()
    }
    fn font_id(&self, _: &Font) -> Result<FontId> {
        Ok(FontId(0))
    }
    fn font_metrics(&self, _: FontId) -> FontMetrics {
        unimplemented!("not used by the warmer")
    }
    fn typographic_bounds(&self, _: FontId, _: GlyphId) -> Result<Bounds<f32>> {
        unimplemented!("not used by the warmer")
    }
    fn advance(&self, _: FontId, _: GlyphId) -> Result<Size<f32>> {
        unimplemented!("not used by the warmer")
    }
    fn glyph_for_char(&self, _: FontId, _: char) -> Option<GlyphId> {
        None
    }
    fn glyph_raster_bounds(&self, params: &RenderGlyphParams) -> Result<Bounds<DevicePixels>> {
        self.bounds_calls.fetch_add(1, Ordering::SeqCst);
        Ok(bounds_of(params.glyph_id.0))
    }
    fn rasterize_glyph(
        &self,
        params: &RenderGlyphParams,
        bounds: Bounds<DevicePixels>,
    ) -> Result<(Size<DevicePixels>, Vec<u8>)> {
        self.rasterised.lock().push(params.clone());
        Ok((bounds.size, bitmap_of(params)))
    }
    fn layout_line(&self, _: &str, _: Pixels, _: &[FontRun]) -> LineLayout {
        LineLayout::default()
    }
    fn recommended_rendering_mode(&self, _: FontId, _: Pixels) -> TextRenderingMode {
        TextRenderingMode::Grayscale
    }
    /// macOS's levels (`gpui_macos`'s formula, font smoothing on).
    fn glyph_dilation_for_color(&self, color: Hsla) -> u8 {
        let rgba: Rgba = color.into();
        let luminance = 0.2126 * rgba.r + 0.7152 * rgba.g + 0.0722 * rgba.b;
        (((4.0 * luminance) + 0.5).floor() as i32).clamp(0, 4) as u8
    }
}

fn glyph(id: u32, dilation: u8) -> RenderGlyphParams {
    RenderGlyphParams {
        font_id: FontId(0),
        glyph_id: GlyphId(id),
        font_size: px(13.),
        subpixel_variant: point(1, 0),
        scale_factor: 2.0,
        is_emoji: false,
        subpixel_rendering: false,
        dilation,
    }
}

fn setup() -> (Arc<Fake>, GlyphWarmer, Arc<dyn PlatformTextSystem>) {
    let fake = Arc::new(Fake::default());
    let warmer = GlyphWarmer::manual(fake.clone());
    let text = warmer.text_system();
    (fake, warmer, text)
}

/// What GPUI's `paint_glyph` does on an atlas miss: the bounds, then the bitmap.
fn draw(text: &Arc<dyn PlatformTextSystem>, params: &RenderGlyphParams) -> Vec<u8> {
    let bounds = text.glyph_raster_bounds(params).unwrap();
    text.rasterize_glyph(params, bounds).unwrap().1
}

fn plan(pairs: &[(u8, u8)]) -> DilationPlan {
    let mut plan = DilationPlan::EMPTY;
    for (from, to) in pairs {
        plan.add(*from, *to);
    }
    plan
}

#[test]
fn a_switch_draws_prepared_glyphs_without_rasterising_in_the_frame() {
    let (fake, warmer, text) = setup();
    draw(&text, &glyph(7, 3));
    warmer.set_plan(plan(&[(3, 0)]));
    assert_eq!(warmer.warm_pending(), 1);
    let rasterised = fake.rasterised.lock().len();
    let bounds_calls = fake.bounds_calls.load(Ordering::SeqCst);

    // The switch: the same glyph at the new theme's level.
    let bitmap = draw(&text, &glyph(7, 0));

    assert_eq!(bitmap, bitmap_of(&glyph(7, 0)), "the platform's own bitmap");
    assert_eq!(
        fake.rasterised.lock().len(),
        rasterised,
        "nothing rasterised"
    );
    assert_eq!(fake.bounds_calls.load(Ordering::SeqCst), bounds_calls);
    let stats = warmer.stats();
    assert_eq!((stats.prepared, stats.served, stats.rasterised), (1, 1, 1));
    assert_eq!(stats.prepared_bytes, 0, "a served glyph is handed over");
}

#[test]
fn glyphs_drawn_under_a_plan_are_queued_as_they_are_drawn() {
    let (fake, warmer, text) = setup();
    warmer.set_plan(plan(&[(3, 0), (3, 1)]));
    draw(&text, &glyph(7, 3));
    draw(&text, &glyph(8, 3));
    draw(&text, &glyph(9, 2));
    assert_eq!(
        warmer.stats().pending,
        4,
        "7 and 8 at levels 0 and 1; 9 not in the plan"
    );
    assert_eq!(warmer.warm_pending(), 4);
    let warmed: Vec<_> = fake.rasterised.lock()[3..]
        .iter()
        .map(|p| (p.glyph_id.0, p.dilation))
        .collect();
    assert_eq!(warmed, [(7, 0), (7, 1), (8, 0), (8, 1)]);
    // Drawing them again (a second frame) queues nothing more.
    draw(&text, &glyph(7, 3));
    assert_eq!(warmer.stats().pending, 0);
}

#[test]
fn a_new_plan_rechecks_what_was_drawn_and_skips_what_is_known() {
    let (_, warmer, text) = setup();
    draw(&text, &glyph(7, 3));
    warmer.set_plan(plan(&[(3, 0)]));
    assert_eq!(warmer.warm_pending(), 1);
    // After the switch the plan goes the other way: 3 is drawn already, nothing to do.
    draw(&text, &glyph(7, 0));
    warmer.set_plan(plan(&[(0, 3)]));
    assert_eq!(warmer.warm_pending(), 0);
    // A plan with a level not seen yet warms it.
    warmer.set_plan(plan(&[(0, 3), (0, 4)]));
    assert_eq!(warmer.warm_pending(), 1);
}

#[test]
fn an_empty_plan_warms_nothing() {
    let (fake, warmer, text) = setup();
    draw(&text, &glyph(7, 0));
    warmer.set_plan(DilationPlan::EMPTY);
    assert_eq!(warmer.warm_pending(), 0);
    assert_eq!(fake.rasterised.lock().len(), 1);
}

#[test]
fn empty_glyphs_emoji_and_glyphs_over_the_budget_are_not_kept() {
    let (_, warmer, text) = setup();
    warmer.set_plan(plan(&[(3, 0)]));
    draw(&text, &glyph(HUGE, 3));
    let empty = glyph(EMPTY, 3);
    text.rasterize_glyph(&empty, bounds_of(EMPTY)).unwrap();
    let emoji = RenderGlyphParams {
        is_emoji: true,
        ..glyph(5, 3)
    };
    draw(&text, &emoji);
    assert_eq!(
        warmer.warm_pending(),
        2,
        "the huge and the empty glyph, not the emoji"
    );
    let stats = warmer.stats();
    assert_eq!(
        (stats.prepared, stats.skipped, stats.prepared_bytes),
        (0, 2, 0)
    );
}

#[test]
fn a_prepared_glyph_asked_for_at_other_bounds_is_rasterised_by_the_platform() {
    let (fake, warmer, text) = setup();
    draw(&text, &glyph(7, 3));
    warmer.set_plan(plan(&[(3, 0)]));
    warmer.warm_pending();
    let other = Bounds::new(
        point(DevicePixels(1), DevicePixels(1)),
        size(DevicePixels(5), DevicePixels(5)),
    );
    let before = fake.rasterised.lock().len();
    let (size, _) = text.rasterize_glyph(&glyph(7, 0), other).unwrap();
    assert_eq!(size, other.size);
    assert_eq!(fake.rasterised.lock().len(), before + 1);
    assert_eq!(warmer.stats().served, 0);
}

#[test]
fn the_worker_thread_prepares_glyphs_off_the_calling_thread() {
    let fake = Arc::new(Fake::default());
    let warmer = GlyphWarmer::new(fake.clone());
    let text = warmer.text_system();
    warmer.set_plan(plan(&[(3, 0)]));
    draw(&text, &glyph(7, 3));
    let deadline = Instant::now() + Duration::from_secs(10);
    while warmer.stats().prepared == 0 {
        assert!(
            Instant::now() < deadline,
            "the worker never prepared the glyph"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let before = fake.rasterised.lock().len();
    assert_eq!(draw(&text, &glyph(7, 0)), bitmap_of(&glyph(7, 0)));
    assert_eq!(fake.rasterised.lock().len(), before);
}

fn levels(plan: DilationPlan, from: u8) -> Vec<u8> {
    (0..LEVELS)
        .filter(|l| plan.targets(from) & (1 << l) != 0)
        .collect()
}

#[test]
fn the_plan_maps_one_dark_text_to_one_light_text_and_back() {
    let fake = Fake::default();
    let level = |c| fake.glyph_dilation_for_color(c);
    let dark = ThemeTokens::fallback(Appearance::Dark);
    let light = ThemeTokens::fallback(Appearance::Light);
    let (dark_text, light_text) = (level(dark.colors.text), level(light.colors.text));
    assert_ne!(dark_text, light_text, "the case this story is about");

    let to_light = DilationPlan::for_switch(dark, [dark, light], level);
    assert!(levels(to_light, dark_text).contains(&light_text));
    let running = level(dark.oxikube.status_running);
    assert!(
        level(light.oxikube.status_running) == running
            || levels(to_light, running).contains(&level(light.oxikube.status_running))
    );
    let to_dark = DilationPlan::for_switch(light, [dark, light], level);
    assert!(levels(to_dark, light_text).contains(&dark_text));
    assert!(DilationPlan::for_switch(dark, [dark], level).is_empty());
}

/// Settings, the bundled themes and the warmer on a scripted text system, the system dark.
fn app(cx: &mut TestAppContext) -> GlyphWarmer {
    let config = tempfile::tempdir().unwrap();
    let warmer = GlyphWarmer::manual(Arc::new(Fake::default()));
    let installed = warmer.clone();
    cx.update(|cx| {
        oxikube_settings::init_with_dir(config.path(), cx);
        SystemAppearance::init(cx);
        SystemAppearance::set(cx, Appearance::Dark);
        install(installed, cx);
        crate::init_with_dir(None, cx);
    });
    warmer
}

#[gpui::test]
fn the_plan_follows_the_active_theme(cx: &mut TestAppContext) {
    let warmer = app(cx);
    let fake = Fake::default();
    let level = |c| fake.glyph_dilation_for_color(c);
    let dark = ThemeTokens::fallback(Appearance::Dark);
    let light = ThemeTokens::fallback(Appearance::Light);
    assert_eq!(
        cx.read(|cx| crate::ActiveTheme::get(cx).name.clone()),
        "One Dark"
    );
    let from_dark = warmer.plan();
    assert!(levels(from_dark, level(dark.colors.text)).contains(&level(light.colors.text)));

    // A `settings.json` edit selecting One Light: the plan now goes back to One Dark.
    cx.update(|cx| {
        SettingsStore::update_global(cx, |store, _| {
            store
                .set_user_settings(r#"{"theme": "One Light"}"#)
                .unwrap()
        })
    });
    assert_eq!(
        cx.read(|cx| crate::ActiveTheme::get(cx).name.clone()),
        "One Light"
    );
    let from_light = warmer.plan();
    assert_ne!(from_light, from_dark);
    assert!(levels(from_light, level(light.colors.text)).contains(&level(dark.colors.text)));
    assert!(cx.read(|cx| cx.has_global::<GlyphWarm>()));
}
