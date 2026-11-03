//! Reading an icon theme: its family JSON and every SVG it names, into memory.
//!
//! Blocking file I/O: call it from the watcher thread or a background task, never the UI thread.
//! Every path goes through [`safe_relative_path`] and the canonical-path check before it is
//! opened; a refused or unreadable icon is dropped (its key falls back to Lucide) and reported.

use super::content::{IconThemeFamilyContent, parse_icon_theme_family};
use super::path::{IconPathError, safe_relative_path};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Icon-theme family files larger than this are refused.
const MAX_FAMILY_BYTES: u64 = 8 * 1024 * 1024;
/// SVG files larger than this are dropped (a real icon is a few kilobytes).
const MAX_ICON_BYTES: u64 = 1024 * 1024;
/// At most this many distinct SVG files are read per family.
const MAX_ICON_FILES: usize = 4096;

/// Why an icon-theme family could not be used at all.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IconThemeError {
    /// Not an icon-theme family file.
    #[error("not an icon theme: {0}")]
    Json(String),
    /// The family has no themes.
    #[error("the icon theme family has no themes")]
    NoThemes,
    /// The file could not be read.
    #[error("cannot read: {0}")]
    Read(String),
    /// The file is over the size limit.
    #[error("file is {0} bytes; icon theme files are limited to 8 MiB")]
    TooLarge(u64),
}

/// A family as read from disk: its content with every icon path normalised (refused and
/// unreadable ones removed) and the SVG bytes keyed by those paths. Mount it with
/// [`IconThemeFamily::mount`](super::IconThemeFamily::mount).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IconThemeSource {
    /// The family, paths normalised.
    pub family: IconThemeFamilyContent,
    /// Normalised relative path to SVG bytes.
    pub files: BTreeMap<String, Arc<[u8]>>,
    /// Icons that were dropped, and why.
    pub problems: Vec<String>,
}

/// Reads the family file at `json_path`, with icon paths relative to `icons_root`.
///
/// # Errors
///
/// When the file is unreadable, too large, not a family, or has no themes.
pub fn read_icon_theme_file(
    json_path: &Path,
    icons_root: &Path,
) -> Result<IconThemeSource, IconThemeError> {
    let read = |err: std::io::Error| IconThemeError::Read(err.to_string());
    let meta = std::fs::metadata(json_path).map_err(read)?;
    if meta.len() > MAX_FAMILY_BYTES {
        return Err(IconThemeError::TooLarge(meta.len()));
    }
    let text = std::fs::read_to_string(json_path).map_err(read)?;
    read_icon_theme(&text, icons_root)
}

/// Parses `text` and reads the SVGs it names from `icons_root`.
///
/// # Errors
///
/// When `text` is not a family or has no themes.
pub fn read_icon_theme(text: &str, icons_root: &Path) -> Result<IconThemeSource, IconThemeError> {
    let mut family = parse_icon_theme_family(text)?;
    if family.themes.is_empty() {
        return Err(IconThemeError::NoThemes);
    }
    let mut reader = Reader::new(icons_root);
    for theme in &mut family.themes {
        for slot in theme.toggle_paths_mut() {
            *slot = slot.take().and_then(|raw| reader.take(&raw));
        }
        let icons = std::mem::take(&mut theme.file_icons);
        theme.file_icons = icons
            .into_iter()
            .filter_map(|(key, mut icon)| {
                icon.path = reader.take(&icon.path)?;
                Some((key, icon))
            })
            .collect();
    }
    Ok(IconThemeSource {
        family,
        files: reader.files,
        problems: reader.problems,
    })
}

/// Reads each distinct icon once and remembers what failed.
struct Reader {
    root: Option<PathBuf>,
    files: BTreeMap<String, Arc<[u8]>>,
    failed: BTreeSet<String>,
    problems: Vec<String>,
}

impl Reader {
    fn new(root: &Path) -> Self {
        Self {
            root: root.canonicalize().ok(),
            files: BTreeMap::new(),
            failed: BTreeSet::new(),
            problems: Vec::new(),
        }
    }

    /// The normalised path of `raw` when its file is (now) in memory.
    fn take(&mut self, raw: &str) -> Option<String> {
        let path = match safe_relative_path(raw) {
            Ok(path) => path,
            Err(err) => {
                self.problems.push(err.to_string());
                return None;
            }
        };
        if self.files.contains_key(&path) {
            return Some(path);
        }
        if self.failed.contains(&path) {
            return None;
        }
        match self.read(&path, raw) {
            Ok(bytes) => {
                self.files.insert(path.clone(), bytes);
                Some(path)
            }
            Err(message) => {
                self.problems.push(message);
                self.failed.insert(path);
                None
            }
        }
    }

    fn read(&self, path: &str, raw: &str) -> Result<Arc<[u8]>, String> {
        if self.files.len() >= MAX_ICON_FILES {
            return Err(format!("{raw}: more than {MAX_ICON_FILES} icons"));
        }
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| format!("{raw}: the icon theme's directory is missing"))?;
        let full = root
            .join(path)
            .canonicalize()
            .map_err(|err| format!("{raw}: {err}"))?;
        if !full.starts_with(root) {
            return Err(IconPathError::Escapes(raw.to_owned()).to_string());
        }
        let meta = std::fs::metadata(&full).map_err(|err| format!("{raw}: {err}"))?;
        if !meta.is_file() || meta.len() > MAX_ICON_BYTES {
            return Err(format!("{raw}: not a file of at most 1 MiB"));
        }
        let bytes = std::fs::read(&full).map_err(|err| format!("{raw}: {err}"))?;
        Ok(Arc::from(bytes))
    }
}

#[cfg(test)]
mod tests;
