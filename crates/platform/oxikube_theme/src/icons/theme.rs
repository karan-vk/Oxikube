//! [`IconThemeFamily`] / [`IconTheme`]: an icon theme ready to draw, its SVGs served by GPUI's
//! asset source.
//!
//! Built from a read [`IconThemeSource`] by mounting its files ([`IconMount`]): every path in the
//! theme becomes an asset path (`icon-themes/<id>/<file>`) that `oxikube_assets::Assets` serves
//! from memory. The theme holds the mount, so the files stay served exactly as long as a registry
//! or the active resolver holds the theme.

use super::content::{DirectoryIconsContent, IconThemeContent, IconThemeFamilyContent};
use super::read::IconThemeSource;
use crate::appearance::Appearance;
use gpui::SharedString;
use oxikube_assets::IconMount;
use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

/// The name of the built-in icon theme: Oxikube's own Lucide icons, the default.
pub const DEFAULT_ICON_THEME: &str = "Lucide";

/// One icon: the asset path its SVG is served at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconDefinition {
    /// Asset path (`icon-themes/<id>/icons/pod.svg`).
    pub path: SharedString,
}

/// A folder's icons, as asset paths; `None` keeps the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirectoryIcons {
    /// Closed folder.
    pub collapsed: Option<SharedString>,
    /// Open folder.
    pub expanded: Option<SharedString>,
}

/// The disclosure chevrons, as asset paths; `None` keeps the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChevronIcons {
    /// Closed (pointing right).
    pub collapsed: Option<SharedString>,
    /// Open (pointing down).
    pub expanded: Option<SharedString>,
}

/// One icon theme, ready to resolve icons from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconTheme {
    /// The name the `icon_theme` setting selects it by.
    pub name: String,
    /// Light or dark.
    pub appearance: Appearance,
    /// A folder's icons.
    pub directory_icons: DirectoryIcons,
    /// Per-folder-name icons.
    pub named_directory_icons: BTreeMap<String, DirectoryIcons>,
    /// The chevrons.
    pub chevron_icons: ChevronIcons,
    /// File name to icon key.
    pub file_stems: BTreeMap<String, String>,
    /// File suffix to icon key.
    pub file_suffixes: BTreeMap<String, String>,
    /// Icon key to icon (Zed's file types plus Oxikube's `kind:` / `status:` / `icon:` keys).
    pub file_icons: BTreeMap<String, IconDefinition>,
    /// The served files; `None` for the built-in theme.
    mount: Option<Arc<IconMount>>,
}

/// A family of icon themes from one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconThemeFamily {
    /// The family's name.
    pub name: String,
    /// Who made it.
    pub author: String,
    /// Its themes.
    pub themes: Vec<IconTheme>,
}

static LUCIDE: LazyLock<Arc<IconTheme>> = LazyLock::new(|| Arc::new(IconTheme::lucide()));

impl IconTheme {
    /// The built-in theme: no overrides, every icon Lucide.
    pub fn lucide() -> Self {
        Self {
            name: DEFAULT_ICON_THEME.to_owned(),
            appearance: Appearance::Dark,
            directory_icons: DirectoryIcons::default(),
            named_directory_icons: BTreeMap::new(),
            chevron_icons: ChevronIcons::default(),
            file_stems: BTreeMap::new(),
            file_suffixes: BTreeMap::new(),
            file_icons: BTreeMap::new(),
            mount: None,
        }
    }

    /// The shared built-in theme.
    pub fn shared_lucide() -> Arc<Self> {
        LUCIDE.clone()
    }

    fn from_content(content: IconThemeContent, mount: Option<&Arc<IconMount>>) -> Self {
        let serve = |path: Option<String>| -> Option<SharedString> {
            let (mount, path) = (mount?, path?);
            mount.contains(&path).then(|| mount.asset_path(&path))
        };
        let directory = |icons: DirectoryIconsContent| DirectoryIcons {
            collapsed: serve(icons.collapsed),
            expanded: serve(icons.expanded),
        };
        Self {
            name: content.name,
            appearance: content.appearance,
            directory_icons: directory(content.directory_icons),
            named_directory_icons: content
                .named_directory_icons
                .into_iter()
                .map(|(name, icons)| (name, directory(icons)))
                .collect(),
            chevron_icons: ChevronIcons {
                collapsed: serve(content.chevron_icons.collapsed),
                expanded: serve(content.chevron_icons.expanded),
            },
            file_stems: content.file_stems,
            file_suffixes: content.file_suffixes,
            file_icons: content
                .file_icons
                .into_iter()
                .filter_map(|(key, icon)| {
                    serve(Some(icon.path)).map(|path| (key, IconDefinition { path }))
                })
                .collect(),
            mount: mount.cloned(),
        }
    }
}

impl IconThemeFamily {
    /// Mounts the files `source` read and builds its themes. Cheap (no I/O): call it on any
    /// thread, typically the UI thread with a source read off it.
    pub fn mount(source: IconThemeSource) -> Self {
        let IconThemeSource { family, files, .. } = source;
        let mount = (!files.is_empty()).then(|| IconMount::new(files));
        Self::from_content(family, mount.as_ref())
    }

    fn from_content(family: IconThemeFamilyContent, mount: Option<&Arc<IconMount>>) -> Self {
        Self {
            name: family.name,
            author: family.author,
            themes: family
                .themes
                .into_iter()
                .map(|theme| IconTheme::from_content(theme, mount))
                .collect(),
        }
    }
}
