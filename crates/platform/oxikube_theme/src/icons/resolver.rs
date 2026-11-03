//! [`IconResolver`]: the asset path of every icon under one icon theme, computed once per theme
//! change so drawing an icon is a lookup and a reference-count bump (no parsing, no allocation).

use super::keys::{IconKey, default_kind_icon, default_status_icon};
use super::theme::IconTheme;
use gpui::SharedString;
use oxikube_assets::IconName;
use std::collections::HashMap;
use std::sync::Arc;

/// The resolved icons of one [`IconTheme`].
#[derive(Debug)]
pub struct IconResolver {
    theme: Arc<IconTheme>,
    /// Indexed by [`IconName::index`].
    icons: Box<[SharedString]>,
    /// `kind:<Kind>`.
    kinds: HashMap<Box<str>, SharedString>,
    /// `kind:<group>/<Kind>`, by group then kind.
    group_kinds: HashMap<Box<str>, HashMap<Box<str>, SharedString>>,
    /// `status:<Status>`.
    statuses: HashMap<Box<str>, SharedString>,
}

impl IconResolver {
    /// Resolves every key of `theme` (see [`super::keys`] for the order).
    pub fn new(theme: Arc<IconTheme>) -> Self {
        let mut icons: Vec<SharedString> = IconName::ALL
            .iter()
            .map(|icon| SharedString::from(icon.path()))
            .collect();
        let mut set = |icon: IconName, path: Option<&SharedString>| {
            if let Some(path) = path {
                icons[icon.index()] = path.clone();
            }
        };
        set(
            IconName::ChevronRight,
            theme.chevron_icons.collapsed.as_ref(),
        );
        set(IconName::ChevronDown, theme.chevron_icons.expanded.as_ref());
        set(IconName::Folder, theme.directory_icons.collapsed.as_ref());
        set(
            IconName::FolderOpen,
            theme.directory_icons.expanded.as_ref(),
        );

        let mut kinds = HashMap::new();
        let mut group_kinds: HashMap<Box<str>, HashMap<Box<str>, SharedString>> = HashMap::new();
        let mut statuses = HashMap::new();
        for (key, icon) in &theme.file_icons {
            let path = icon.path.clone();
            match IconKey::parse(key) {
                IconKey::Kind { group: None, kind } => {
                    kinds.insert(kind.into(), path);
                }
                IconKey::Kind {
                    group: Some(group),
                    kind,
                } => {
                    group_kinds
                        .entry(group.into())
                        .or_default()
                        .insert(kind.into(), path);
                }
                IconKey::Status(status) => {
                    statuses.insert(status.into(), path);
                }
                IconKey::Icon(name) => set(name, Some(&path)),
                IconKey::File(_) => {}
            }
        }
        Self {
            theme,
            icons: icons.into_boxed_slice(),
            kinds,
            group_kinds,
            statuses,
        }
    }

    /// The resolver of the built-in Lucide theme.
    pub fn lucide() -> Self {
        Self::new(IconTheme::shared_lucide())
    }

    /// The theme this resolves.
    pub fn theme(&self) -> &Arc<IconTheme> {
        &self.theme
    }

    /// The asset path `icon` is drawn from.
    pub fn icon(&self, icon: IconName) -> SharedString {
        self.icons[icon.index()].clone()
    }

    /// The asset path of the icon of `kind` in API `group` (empty for core): the theme's
    /// `kind:<group>/<Kind>`, then `kind:<Kind>`, then the kind's Lucide default.
    pub fn kind(&self, group: &str, kind: &str) -> SharedString {
        self.group_kinds
            .get(group)
            .and_then(|kinds| kinds.get(kind))
            .or_else(|| self.kinds.get(kind))
            .cloned()
            .unwrap_or_else(|| self.icon(default_kind_icon(group, kind)))
    }

    /// The asset path of the icon of the status text `status`: the theme's `status:<Status>`,
    /// then the status's Lucide default.
    pub fn status(&self, status: &str) -> SharedString {
        self.statuses
            .get(status)
            .cloned()
            .unwrap_or_else(|| self.icon(default_status_icon(status)))
    }

    /// The asset path of a file's icon, Zed's way: its whole name in `file_stems`, then each
    /// suffix from the longest (`values.yaml.tmpl`: `yaml.tmpl`, then `tmpl`) in `file_suffixes`.
    /// `None` when the theme has no icon for it.
    pub fn file(&self, file_name: &str) -> Option<SharedString> {
        let theme = &self.theme;
        let suffixes = file_name
            .match_indices('.')
            .map(|(ix, _)| &file_name[ix + 1..]);
        let key = theme.file_stems.get(file_name).or_else(|| {
            suffixes
                .filter(|suffix| !suffix.is_empty())
                .find_map(|suffix| theme.file_suffixes.get(suffix))
        })?;
        theme.file_icons.get(key).map(|icon| icon.path.clone())
    }
}

impl Default for IconResolver {
    fn default() -> Self {
        Self::lucide()
    }
}

#[cfg(test)]
mod tests;
