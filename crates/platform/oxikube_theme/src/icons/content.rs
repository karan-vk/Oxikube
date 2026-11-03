//! The icon-theme file format: Zed's icon-theme family JSON, as written.
//!
//! Modelled from the documented file format (Zed's `icon_themes/v0.2.0` schema), not copied from
//! Zed's GPL `theme` crate. A family file holds one or more themes; every theme maps folders,
//! chevrons and file names to icon keys, and `file_icons` maps keys to SVG paths relative to the
//! icon theme's directory. Oxikube's Kubernetes keys (`kind:Pod`, `status:Running`, `icon:box`)
//! are ordinary `file_icons` keys, so a Zed icon theme loads unchanged (see [`super::keys`]).
//!
//! ```json
//! {
//!   "$schema": "https://zed.dev/schema/icon_themes/v0.2.0.json",
//!   "name": "Material", "author": "...",
//!   "themes": [{
//!     "name": "Material", "appearance": "dark",
//!     "directory_icons": { "collapsed": "./icons/folder.svg", "expanded": "./icons/folder-open.svg" },
//!     "chevron_icons": { "collapsed": "./icons/chevron-right.svg", "expanded": "./icons/chevron-down.svg" },
//!     "file_stems": { "Dockerfile": "docker" },
//!     "file_suffixes": { "yaml": "yaml" },
//!     "file_icons": {
//!       "docker": { "path": "./icons/docker.svg" },
//!       "kind:Pod": { "path": "./icons/pod.svg" }
//!     }
//!   }]
//! }
//! ```

use super::IconThemeError;
use crate::appearance::Appearance;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A whole icon-theme file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconThemeFamilyContent {
    /// The schema URL the file names, if any (not checked).
    #[serde(rename = "$schema", default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// The family's name.
    pub name: String,
    /// Who made it.
    #[serde(default)]
    pub author: String,
    /// The themes in the family.
    pub themes: Vec<IconThemeContent>,
}

/// One icon theme of a family.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconThemeContent {
    /// The name the `icon_theme` setting selects it by.
    pub name: String,
    /// Light or dark (dark when unset).
    #[serde(default)]
    pub appearance: Appearance,
    /// The icons of a folder, closed and open.
    #[serde(default)]
    pub directory_icons: DirectoryIconsContent,
    /// Per-folder-name icons, closed and open.
    #[serde(default)]
    pub named_directory_icons: BTreeMap<String, DirectoryIconsContent>,
    /// The disclosure chevrons, closed and open.
    #[serde(default)]
    pub chevron_icons: ChevronIconsContent,
    /// File name (`Dockerfile`) to icon key.
    #[serde(default)]
    pub file_stems: BTreeMap<String, String>,
    /// File suffix (`yaml`, `tar.gz`) to icon key.
    #[serde(default)]
    pub file_suffixes: BTreeMap<String, String>,
    /// Icon key to its SVG.
    #[serde(default)]
    pub file_icons: BTreeMap<String, IconDefinitionContent>,
}

/// A folder's icons.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryIconsContent {
    /// SVG path for a closed folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<String>,
    /// SVG path for an open folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<String>,
}

/// The chevrons' icons.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChevronIconsContent {
    /// SVG path for a closed disclosure (pointing right).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collapsed: Option<String>,
    /// SVG path for an open disclosure (pointing down).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<String>,
}

/// One icon: an SVG path relative to the icon theme's directory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconDefinitionContent {
    /// The SVG file, e.g. `./icons/pod.svg`.
    pub path: String,
}

impl IconThemeContent {
    /// The folder and chevron SVG paths, mutably (the `file_icons` paths are not optional, so
    /// they are walked apart): what reading the theme validates and rewrites.
    pub(crate) fn toggle_paths_mut(&mut self) -> impl Iterator<Item = &mut Option<String>> {
        [&mut self.directory_icons]
            .into_iter()
            .chain(self.named_directory_icons.values_mut())
            .flat_map(|icons| [&mut icons.collapsed, &mut icons.expanded])
            .chain([
                &mut self.chevron_icons.collapsed,
                &mut self.chevron_icons.expanded,
            ])
    }
}

/// Parses a family file: JSON with comments and trailing commas, as Zed reads it.
///
/// # Errors
///
/// [`IconThemeError::Json`] when the text is not a family file.
pub fn parse_icon_theme_family(text: &str) -> Result<IconThemeFamilyContent, IconThemeError> {
    let json = |err: serde_json_lenient::Error| IconThemeError::Json(err.to_string());
    let mut deserializer = serde_json_lenient::Deserializer::from_str(text);
    let family = IconThemeFamilyContent::deserialize(&mut deserializer).map_err(json)?;
    deserializer.end().map_err(json)?;
    Ok(family)
}
