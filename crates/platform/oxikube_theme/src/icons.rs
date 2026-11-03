//! Icon themes (E22-S11): Zed's icon-theme format, mapped onto Kubernetes kinds and statuses,
//! with Oxikube's Lucide icons as the built-in default.
//!
//! An icon theme never changes which [`IconName`](oxikube_assets::IconName) a view asks for; it
//! changes the asset path that name (or a kind, or a status) is drawn from. The flow:
//!
//! 1. [`read`]: a family file and the SVGs it names are read into memory off the UI thread, every
//!    path checked to stay inside the icon theme's directory ([`path`]).
//! 2. [`theme`]: [`IconThemeFamily::mount`] serves those bytes through `oxikube_assets::Assets`
//!    ([`IconMount`](oxikube_assets::IconMount)) and turns paths into asset paths.
//! 3. The [`ThemeRegistry`](crate::ThemeRegistry) keeps icon themes by source, like colour
//!    themes; the `icon_theme` setting ([`settings`]) picks one.
//! 4. [`resolver`]: [`IconResolver`] resolves every icon, kind and status key once per theme
//!    change ([`keys`] has the `kind:` / `status:` / `icon:` convention and the Lucide defaults).
//! 5. [`active`]: the [`ActiveIconTheme`] global and its per-thread copy that `oxikube_ui::Icon`
//!    reads while drawing.
//!
//! The user's icon themes live in `<config>/icon_themes/` ([`user_dir`]); extensions (E23-S06)
//! load theirs with [`load_icon_theme`] under their own `ThemeSource::Extension`.

pub mod active;
pub mod content;
pub mod keys;
pub mod path;
pub mod read;
pub mod resolver;
pub mod settings;
pub mod theme;
pub mod user_dir;

pub use active::{
    ActiveIconTheme, apply_user_icon_scan, icon_path, init_icon_themes, init_icon_themes_with_dir,
    kind_icon_path, load_icon_theme, refresh_active_icon_theme, remove_icon_themes,
    status_icon_path, with_active_icons,
};
pub use content::{
    ChevronIconsContent, DirectoryIconsContent, IconDefinitionContent, IconThemeContent,
    IconThemeFamilyContent, parse_icon_theme_family,
};
pub use keys::{IconKey, default_kind_icon, default_status_icon};
pub use path::{IconPathError, safe_relative_path};
pub use read::{IconThemeError, IconThemeSource, read_icon_theme, read_icon_theme_file};
pub use resolver::IconResolver;
pub use settings::IconThemeSettings;
pub use theme::{
    ChevronIcons, DEFAULT_ICON_THEME, DirectoryIcons, IconDefinition, IconTheme, IconThemeFamily,
};
pub use user_dir::{UserIconThemes, icon_themes_dir, scan_icon_themes_dir};
