//! `glyph_warm::wrap_platform` on the OS platform: GPUI gets the warmer's text system, everything
//! else is the platform's own. A `harness = false` test, because AppKit makes the platform only on
//! the main thread.
#![allow(clippy::print_stdout)]

use gpui::PlatformTextSystem;
use oxikube_theme::glyph_warm::wrap_platform;
use std::sync::Arc;

fn address(text: Arc<dyn PlatformTextSystem>) -> *const () {
    Arc::as_ptr(&text) as *const ()
}

fn main() {
    let inner = gpui_platform::current_platform(true);
    let (platform, warmer) = wrap_platform(inner.clone());
    assert_eq!(
        address(platform.text_system()),
        address(warmer.text_system())
    );
    assert_ne!(
        address(platform.text_system()),
        address(inner.text_system())
    );
    assert_eq!(platform.compositor_name(), inner.compositor_name());
    assert_eq!(platform.window_appearance(), inner.window_appearance());
    assert_eq!(platform.active_window(), inner.active_window());
    // The decorated text system answers with the platform's fonts.
    let font = gpui::font(".SystemUIFont");
    assert_eq!(
        platform.text_system().font_id(&font).ok(),
        inner.text_system().font_id(&font).ok()
    );
    println!("test the_wrapped_platform_hands_gpui_the_warmers_text_system ... ok");
}
