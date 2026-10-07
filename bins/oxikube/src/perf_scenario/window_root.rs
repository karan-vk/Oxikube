//! The window root every view-driving scenario mounts its view in (#509).
//!
//! A scenario measures what the app draws, so its view sits where the app puts one: inside
//! `oxikube_ui`'s window root (`oxikube_ui::root::new_root`, the gpui-component `Root` the main
//! window is built with), behind the `--perf` frame hook when probing. The root is what gives the
//! window's text the theme's UI family, which gpui-component has already resolved to a family the
//! machine has. A view drawn outside it asks for GPUI's default family, `.SystemUIFont`, on every
//! text run. On Linux that maps to IBM Plex Sans, which most machines (the CI runners included) do
//! not install, so GPUI walks its fallback stack on every run and builds a fresh error for each
//! family it misses: about nine per run before DejaVu Sans. With `RUST_BACKTRACE=1` (which
//! `cargo xtask` used to hand down from `.cargo/config.toml`) each of those errors also captured a
//! stack trace, and a frame of the pods table took about 300 ms on the Linux runner.
//!
//! [`TextFont::check`] fails the sample when the family the view's text is drawn in is not one
//! the machine resolves directly, so a scenario that mounts its view some other way fails instead
//! of measuring the fallback.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use gpui::{
    AnyView, App, AppContext as _, Context, Entity, Font, IntoElement, Render, SharedString, Window,
};
use oxikube_runtime::perf::{PerfRoot, Recorder};
use oxikube_ui::root::{Root, new_root};

/// Builds the window root around `view` as the app does: `oxikube_ui`'s root, then (when `hook` is
/// given) the `--perf` frame hook, then the view. Call from the closure given to
/// `cx.open_window`. The returned [`TextFont`] reads the font the view's text is drawn in.
pub(super) fn mount(
    view: AnyView,
    hook: Option<Arc<Recorder>>,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<Root>, TextFont) {
    let content: AnyView = match hook {
        Some(recorder) => cx.new(|_| PerfRoot::new(view, recorder)).into(),
        None => view,
    };
    let (probe, font) = probe(content, cx);
    (new_root(probe, window, cx), font)
}

/// Wraps `content` in a view that records the window's text font when it renders.
fn probe(content: AnyView, cx: &mut App) -> (Entity<FontProbe>, TextFont) {
    let seen = Rc::new(RefCell::new(None));
    let probe = cx.new(|_| FontProbe {
        content,
        seen: seen.clone(),
    });
    (probe, TextFont { seen })
}

/// The font the window gives the scenario's view, as seen by the view's parent while it draws.
pub(super) struct TextFont {
    seen: Rc<RefCell<Option<Font>>>,
}

impl TextFont {
    /// Fails unless the view's text is drawn in a family the text system resolves directly.
    /// Call after the window has drawn once.
    pub(super) fn check(&self, cx: &App) -> Result<()> {
        let font = self
            .seen
            .borrow()
            .clone()
            .context("the scenario window has not drawn its view yet")?;
        let resolved = resolved_family(&font, cx);
        ensure!(
            resolved.as_ref() == Some(&font.family),
            "the view's text asks for the font family {:?}, which this machine does not have: \
             GPUI falls back to {:?} on every text run, so the frames would measure the fallback \
             (#509). Mount the view with `window_root::mount`, as the app's window does.",
            font.family,
            resolved.as_deref().unwrap_or("nothing"),
        );
        Ok(())
    }
}

/// The family GPUI draws `font` in: its own when the text system loads it, else the first
/// family of GPUI's fallback stack that loads.
fn resolved_family(font: &Font, cx: &App) -> Option<SharedString> {
    let text = cx.text_system();
    text.get_font_for_id(text.resolve_font(font))
        .map(|resolved| resolved.family)
}

/// Records the window's text font when it renders, then renders the content unchanged.
struct FontProbe {
    content: AnyView,
    seen: Rc<RefCell<Option<Font>>>,
}

impl Render for FontProbe {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut seen = self.seen.borrow_mut();
        if seen.is_none() {
            *seen = Some(window.text_style().font());
        }
        self.content.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        AnyWindowHandle, Bounds, DevicePixels, FontId, FontMetrics, FontRun, GlyphId,
        HeadlessAppContext, LineLayout, NoopTextSystem, Pixels, PlatformTextSystem,
        RenderGlyphParams, Size, TextRenderingMode, div, px, size,
    };
    use std::borrow::Cow;

    /// The families of the fake machine: a Linux CI runner's, without IBM Plex Sans (what GPUI
    /// maps `.SystemUIFont` to on Linux) or any other family of GPUI's fallback stack before
    /// DejaVu Sans.
    const INSTALLED: &[&str] = &["DejaVu Sans", "DejaVu Sans Mono"];

    /// A text system that loads only [`INSTALLED`] families, each with its own id, and fails the
    /// rest as the platform text systems do; everything else is GPUI's no-op text system.
    struct Machine(NoopTextSystem);

    impl PlatformTextSystem for Machine {
        fn add_fonts(&self, fonts: Vec<Cow<'static, [u8]>>) -> anyhow::Result<()> {
            self.0.add_fonts(fonts)
        }
        fn all_font_names(&self) -> Vec<String> {
            INSTALLED.iter().map(|name| (*name).to_owned()).collect()
        }
        fn font_id(&self, descriptor: &Font) -> anyhow::Result<FontId> {
            INSTALLED
                .iter()
                .position(|name| *name == descriptor.family.as_ref())
                .map(|ix| FontId(ix + 1))
                .with_context(|| format!("no font family {:?}", descriptor.family))
        }
        fn font_metrics(&self, font_id: FontId) -> FontMetrics {
            self.0.font_metrics(font_id)
        }
        fn typographic_bounds(
            &self,
            font_id: FontId,
            glyph_id: GlyphId,
        ) -> anyhow::Result<Bounds<f32>> {
            self.0.typographic_bounds(font_id, glyph_id)
        }
        fn advance(&self, font_id: FontId, glyph_id: GlyphId) -> anyhow::Result<Size<f32>> {
            self.0.advance(font_id, glyph_id)
        }
        fn glyph_for_char(&self, font_id: FontId, ch: char) -> Option<GlyphId> {
            self.0.glyph_for_char(font_id, ch)
        }
        fn glyph_raster_bounds(
            &self,
            params: &RenderGlyphParams,
        ) -> anyhow::Result<Bounds<DevicePixels>> {
            self.0.glyph_raster_bounds(params)
        }
        fn rasterize_glyph(
            &self,
            params: &RenderGlyphParams,
            raster_bounds: Bounds<DevicePixels>,
        ) -> anyhow::Result<(Size<DevicePixels>, Vec<u8>)> {
            self.0.rasterize_glyph(params, raster_bounds)
        }
        fn layout_line(&self, text: &str, font_size: Pixels, runs: &[FontRun]) -> LineLayout {
            self.0.layout_line(text, font_size, runs)
        }
        fn recommended_rendering_mode(
            &self,
            font_id: FontId,
            font_size: Pixels,
        ) -> TextRenderingMode {
            self.0.recommended_rendering_mode(font_id, font_size)
        }
    }

    /// A headless app on the fake machine, with the component library (and its theme) set up as
    /// the scenarios do. No renderer: nothing here needs pixels.
    fn app() -> HeadlessAppContext {
        let mut cx = HeadlessAppContext::with_asset_source(
            Arc::new(Machine(NoopTextSystem)),
            Arc::new(oxikube_ui::Assets),
        );
        cx.update(|cx| {
            oxikube_ui::init(cx);
            oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        });
        cx
    }

    /// A view with nothing to draw.
    struct Empty;

    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    const WINDOW: Size<Pixels> = size(px(400.), px(300.));

    /// Opens a window whose root `build` makes around an empty view, draws it once and returns
    /// the view's text font.
    fn draw(
        cx: &mut HeadlessAppContext,
        build: impl FnOnce(AnyView, &mut Window, &mut App) -> (AnyView, TextFont),
    ) -> TextFont {
        let mut text_font = None;
        let window: AnyWindowHandle = cx
            .open_window(WINDOW, |window, cx| {
                let view = cx.new(|_| Empty);
                let (root, font) = build(view.into(), window, cx);
                text_font = Some(font);
                cx.new(|_| Wrap(root))
            })
            .expect("open the window")
            .into();
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .expect("draw");
        text_font.expect("built")
    }

    /// A window root over any view.
    struct Wrap(AnyView);

    impl Render for Wrap {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.0.clone()
        }
    }

    fn seen_family(font: &TextFont) -> SharedString {
        font.seen
            .borrow()
            .clone()
            .expect("the probe rendered")
            .family
    }

    #[test]
    fn a_view_mounted_as_the_app_mounts_it_is_drawn_in_an_installed_family() {
        let mut cx = app();
        let font = draw(&mut cx, |view, window, cx| {
            let (root, font) = mount(view, None, window, cx);
            (root.into(), font)
        });
        // gpui-component named the installed family `.SystemUIFont` lands on, and the root gives
        // it to the view.
        assert_eq!(seen_family(&font), "DejaVu Sans");
        cx.update(|cx| font.check(cx))
            .expect("the family resolves directly");
    }

    #[test]
    fn a_view_drawn_outside_the_root_falls_back_on_every_run_and_fails_the_check() {
        let mut cx = app();
        let font = draw(&mut cx, |view, _, cx| {
            let (probe, font) = probe(view, cx);
            (probe.into(), font)
        });
        assert_eq!(seen_family(&font), ".SystemUIFont");
        let err = cx
            .update(|cx| font.check(cx))
            .expect_err("GPUI's default family is not installed")
            .to_string();
        assert!(err.contains("\".SystemUIFont\""), "{err}");
        assert!(err.contains("\"DejaVu Sans\""), "{err}");
    }

    #[test]
    fn checking_before_the_first_draw_fails() {
        let mut cx = app();
        let font = TextFont {
            seen: Rc::new(RefCell::new(None)),
        };
        assert!(cx.update(|cx| font.check(cx)).is_err());
    }
}
