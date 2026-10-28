//! `glyph_warm::wrap_platform` on the OS platform: GPUI gets the warmer's text system, everything
//! else is the platform's own. A `harness = false` test, because AppKit makes the platform only on
//! the main thread.
#![allow(clippy::print_stdout, reason = "a test reports on stdout")]

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
        address(warmer.text_system()),
        "GPUI must get the warmer's text system"
    );
    assert_ne!(
        address(platform.text_system()),
        address(inner.text_system()),
        "the wrapped platform must not hand out the inner text system unwrapped"
    );
    assert_eq!(
        platform.compositor_name(),
        inner.compositor_name(),
        "compositor name is forwarded to the OS platform"
    );
    assert_eq!(
        platform.window_appearance(),
        inner.window_appearance(),
        "window appearance is forwarded to the OS platform"
    );
    assert_eq!(
        platform.active_window(),
        inner.active_window(),
        "active window is forwarded to the OS platform"
    );
    // The decorated text system answers with the platform's fonts.
    let font = gpui::font(".SystemUIFont");
    assert_eq!(
        platform.text_system().font_id(&font).ok(),
        inner.text_system().font_id(&font).ok(),
        "the warmer's text system resolves fonts like the platform's own"
    );
    println!("test the_wrapped_platform_hands_gpui_the_warmers_text_system ... ok");
}
