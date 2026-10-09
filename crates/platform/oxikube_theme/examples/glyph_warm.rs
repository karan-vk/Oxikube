//! What a switch from One Dark to One Light costs the frame in glyph rasterisation, with and
//! without the warm-up (E05-P602), on the platform's real text system (CoreText on macOS).
//!
//! A screen of the pods table (60 rows: name, namespace, status, restarts, age, node, IP) is laid
//! out in the system UI font at 13 px and scale 2, and every glyph GPUI would paint is drawn at One
//! Dark's text level first (the screen before the switch). Then the One Light frame:
//!
//! - without warm-up: the platform rasterises every glyph again at One Light's level, in the frame;
//! - with warm-up: the worker's job (timed, off the frame) has prepared them; the frame's bounds
//!   and bitmap requests are answered from the prepared store.
//!
//! The atlas upload (a copy into a Metal texture) is the same in both and not included.
//!
//! `cargo run --release -p oxikube_theme --example glyph_warm`
#![allow(clippy::print_stdout)]

use gpui::{
    Font, FontRun, PlatformTextSystem, RenderGlyphParams, SUBPIXEL_VARIANTS_X, font, point, px,
};
use oxikube_theme::glyph_warm::{DilationPlan, GlyphWarmer};
use oxikube_theme::{Appearance, ThemeTokens};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SCALE: f32 = 2.0;

/// Every distinct glyph a screen of the pods table paints, as `Window::paint_glyph` keys it.
fn screen_glyphs(text: &dyn PlatformTextSystem, ui: &Font) -> Vec<RenderGlyphParams> {
    let font_id = text.font_id(ui).expect("the system UI font");
    let columns = [16.0, 330.0, 470.0, 560.0, 630.0, 700.0, 960.0];
    let mut seen = HashSet::new();
    let mut glyphs = Vec::new();
    for row in 0..60u32 {
        let cells = [
            format!(
                "load-{:03}-7f9c8d{:04x}-x{:02}",
                row % 997,
                row * 7919 % 65_536,
                row % 97
            ),
            format!("perf-ns-{}", row % 8),
            ["Running", "Pending", "Succeeded", "CrashLoopBackOff"][row as usize % 4].to_string(),
            format!("{}", row % 5),
            format!("{}m{}s", row % 59, (row * 13) % 60),
            "kind-oxikube-control-plane".to_string(),
            format!("10.244.{}.{}", row % 4, row % 251),
        ];
        for (cell, x) in cells.iter().zip(columns) {
            let runs = [FontRun {
                len: cell.len(),
                font_id,
            }];
            let layout = text.layout_line(cell, px(13.), &runs);
            for run in &layout.runs {
                for glyph in &run.glyphs {
                    let x = (x + f32::from(glyph.position.x)) * SCALE;
                    let quantized =
                        (x * SUBPIXEL_VARIANTS_X as f32).round() / SUBPIXEL_VARIANTS_X as f32;
                    let params = RenderGlyphParams {
                        font_id: run.font_id,
                        glyph_id: glyph.id,
                        font_size: px(13.),
                        subpixel_variant: point(
                            (quantized.fract() * SUBPIXEL_VARIANTS_X as f32) as u8,
                            0,
                        ),
                        scale_factor: SCALE,
                        is_emoji: glyph.is_emoji,
                        subpixel_rendering: false,
                        dilation: 0,
                    };
                    if !params.is_emoji && seen.insert(format!("{params:?}")) {
                        glyphs.push(params);
                    }
                }
            }
        }
    }
    glyphs
}

fn at(params: &RenderGlyphParams, dilation: u8) -> RenderGlyphParams {
    RenderGlyphParams {
        dilation,
        ..params.clone()
    }
}

/// One frame's worth of GPUI's atlas misses: bounds, then bitmap, for every glyph.
fn frame(text: &dyn PlatformTextSystem, glyphs: &[RenderGlyphParams], dilation: u8) -> Duration {
    let start = Instant::now();
    for params in glyphs {
        let params = at(params, dilation);
        let bounds = text.glyph_raster_bounds(&params).expect("bounds");
        if !bounds.is_empty() {
            text.rasterize_glyph(&params, bounds).expect("bitmap");
        }
    }
    start.elapsed()
}

fn main() {
    let platform = gpui_platform::current_platform(true);
    let inner: Arc<dyn PlatformTextSystem> = platform.text_system();
    let glyphs = screen_glyphs(&*inner, &font(".SystemUIFont"));
    let dark = ThemeTokens::fallback(Appearance::Dark);
    let light = ThemeTokens::fallback(Appearance::Light);
    let level = |color| inner.glyph_dilation_for_color(color);
    let (from, to) = (level(dark.colors.text), level(light.colors.text));
    println!(
        "{} distinct glyphs on screen; One Dark text at level {from}, One Light text at level {to}",
        glyphs.len()
    );
    if from == to {
        println!("this platform draws both at one level: nothing to warm");
        return;
    }

    // Without warm-up: the screen in One Dark, then the One Light frame rasterises it again.
    frame(&*inner, &glyphs, from);
    let cold = frame(&*inner, &glyphs, to);
    println!("switch frame, no warm-up:   {cold:>10.2?} rasterising in the frame");

    // With warm-up, on a fresh decorated text system over the same platform.
    let warmer = GlyphWarmer::manual(inner.clone());
    let text = warmer.text_system();
    frame(&*text, &glyphs, from);
    // The plan is remade on the UI thread at every switch (`glyph_warm::install`'s observer).
    let mut plans: Vec<Duration> = (0..200)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(DilationPlan::for_switch(dark, [dark, light], level));
            start.elapsed()
        })
        .collect();
    plans.sort();
    println!(
        "plan for a switch (UI thread): median {:>8.2?}, worst {:>8.2?}",
        plans[plans.len() / 2],
        plans[plans.len() - 1]
    );
    warmer.set_plan(DilationPlan::for_switch(dark, [dark, light], level));
    let start = Instant::now();
    let jobs = warmer.warm_pending();
    let worker = start.elapsed();
    let kept = warmer.stats().prepared_bytes;
    let warm = frame(&*text, &glyphs, to);
    let stats = warmer.stats();
    println!(
        "warm-up worker (off frame): {worker:>10.2?} for {jobs} glyphs ({} KiB kept)",
        kept.div_ceil(1024)
    );
    println!(
        "switch frame, warmed:       {warm:>10.2?} ({} served, {} rasterised in the frame)",
        stats.served,
        stats.rasterised - glyphs.len() as u64
    );
}
