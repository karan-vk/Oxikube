//! The GPUI asset source for the embedded icons and the mounted icon-theme files.

use crate::icon_mount;
use crate::icons::IconName;
use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Serves the icons in [`IconName`] at `icons/<stem>.svg`, and the files of live
/// [`IconMount`](crate::IconMount)s (an icon theme's SVGs, already in memory) at
/// `icon-themes/<id>/<path>`.
///
/// Unknown paths return `Ok(None)` rather than an error so this source can be chained in front of
/// another one (`oxikube_ui::Assets` falls back to the component library's bundle).
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(icon) = IconName::from_path(path) {
            return Ok(Some(Cow::Borrowed(icon.svg())));
        }
        Ok(icon_mount::load(path))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all: Vec<SharedString> = IconName::ALL
            .iter()
            .map(|icon| icon.path())
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect();
        all.extend(icon_mount::list(path));
        Ok(all)
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
    fn serves_mounted_icon_theme_files() {
        let mount = crate::IconMount::new([(
            "icons/pod.svg".to_owned(),
            std::sync::Arc::from(&b"<svg>pod</svg>"[..]),
        )]);
        let path = mount.asset_path("icons/pod.svg");
        assert_eq!(
            Assets.load(&path).unwrap().as_deref(),
            Some(&b"<svg>pod</svg>"[..])
        );
        assert!(Assets.list("icon-themes/").unwrap().contains(&path));
    }

    #[test]
    fn lists_by_prefix() {
        let all = Assets.list("icons/").unwrap();
        assert_eq!(all.len(), IconName::ALL.len());
        assert!(Assets.list("fonts/").unwrap().is_empty());
    }
}
