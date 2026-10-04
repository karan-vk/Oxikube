//! The GPUI asset source for the embedded icons.

use crate::icons::IconName;
use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Serves the icons in [`IconName`] at `icons/<stem>.svg`.
///
/// Unknown paths return `Ok(None)` rather than an error so this source can be chained in front of
/// another one (`oxikube_ui::Assets` falls back to the component library's bundle).
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(IconName::from_path(path).map(|icon| Cow::Borrowed(icon.svg())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(IconName::ALL
            .iter()
            .map(|icon| icon.path())
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_embedded_icons_and_misses_cleanly() {
        let assets = Assets;
        let svg = assets.load("icons/box.svg").unwrap().expect("box exists");
        assert!(svg.starts_with(b"<svg"));
        assert!(assets.load("icons/nope.svg").unwrap().is_none());
    }

    #[test]
    fn lists_by_prefix() {
        let all = Assets.list("icons/").unwrap();
        assert_eq!(all.len(), IconName::ALL.len());
        assert!(Assets.list("fonts/").unwrap().is_empty());
    }
}
