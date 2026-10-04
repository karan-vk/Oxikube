//! The application asset source: Oxikube's icons first, then the component library's bundle.

use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Serves [`oxikube_assets::Assets`] (our icons) and falls back to gpui-component's own bundle for
/// the glyphs its widgets draw themselves (chevrons, close buttons, checkmarks).
///
/// GPUI fixes the asset source when the `Application` is built, so [`crate::init`] cannot install
/// it. The bin registers it once: `gpui_platform::application().with_assets(oxikube_ui::Assets)`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = oxikube_assets::Assets.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit_assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all = oxikube_assets::Assets.list(path)?;
        for extra in gpui_kit_assets::Assets.list(path)? {
            if !all.contains(&extra) {
                all.push(extra);
            }
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_our_icons_and_the_library_bundle() {
        // `box` is ours; `chevron-down` is in both (ours wins); `close` only the library ships and
        // stays reachable through the fallback.
        assert!(
            oxikube_assets::Assets
                .load("icons/close.svg")
                .unwrap()
                .is_none()
        );
        assert!(Assets.load("icons/box.svg").unwrap().is_some());
        assert!(Assets.load("icons/chevron-down.svg").unwrap().is_some());
        assert!(Assets.load("icons/close.svg").unwrap().is_some());
        assert!(Assets.load("icons/definitely-not-an-icon.svg").is_err());
    }
}
