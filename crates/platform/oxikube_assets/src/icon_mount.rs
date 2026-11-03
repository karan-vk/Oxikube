//! Icon files that are not embedded: the SVGs of a user's (later an extension's) icon theme.
//!
//! GPUI fixes the asset source when the `Application` is built, before any icon theme is read, so
//! [`Assets`](crate::Assets) serves these from a process-wide table that icon themes add to at
//! run time. The bytes are read off the UI thread by whoever loads the theme (`oxikube_theme`'s
//! icon-theme scan) and handed over here already in memory: serving one never touches the disk.
//!
//! Each loaded set of files is an [`IconMount`] with its own id, served under
//! `icon-themes/<id>/<relative path>`. A reload mounts the new files under a new id, so GPUI's
//! sprite atlas (keyed by path) never shows a stale picture of an edited file. Dropping the last
//! [`Arc<IconMount>`] (the theme that referenced it was replaced or removed) frees its bytes.

use gpui::SharedString;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

/// The path prefix mounted icon files are served under.
pub const ICON_MOUNT_PREFIX: &str = "icon-themes/";

type Files = HashMap<Box<str>, Arc<[u8]>>;

static MOUNTS: LazyLock<RwLock<HashMap<u64, Files>>> = LazyLock::new(RwLock::default);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// One set of icon files in memory, served while it is alive.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct IconMount {
    id: u64,
}

impl IconMount {
    /// Mounts `files` (relative path, SVG bytes) under a fresh id. Relative paths use `/` and are
    /// already validated by the caller (no `..`, not absolute).
    pub fn new(files: impl IntoIterator<Item = (String, Arc<[u8]>)>) -> Arc<Self> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let files: Files = files
            .into_iter()
            .map(|(path, bytes)| (path.into_boxed_str(), bytes))
            .collect();
        MOUNTS
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, files);
        Arc::new(Self { id })
    }

    /// The asset path `relative` is served under, e.g. `icon-themes/3/icons/pod.svg`.
    pub fn asset_path(&self, relative: &str) -> SharedString {
        format!("{ICON_MOUNT_PREFIX}{}/{relative}", self.id).into()
    }

    /// Whether `relative` is one of this mount's files.
    pub fn contains(&self, relative: &str) -> bool {
        MOUNTS
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&self.id)
            .is_some_and(|files| files.contains_key(relative))
    }
}

impl Drop for IconMount {
    fn drop(&mut self) {
        MOUNTS
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

/// The bytes served at `path`, when it names a file of a live mount.
pub(crate) fn load(path: &str) -> Option<Cow<'static, [u8]>> {
    let (id, relative) = path.strip_prefix(ICON_MOUNT_PREFIX)?.split_once('/')?;
    let id: u64 = id.parse().ok()?;
    let mounts = MOUNTS.read().unwrap_or_else(PoisonError::into_inner);
    let bytes = mounts.get(&id)?.get(relative)?;
    // GPUI wants `'static` bytes; a copy per rasterisation (once per path and size) is cheap.
    Some(Cow::Owned(bytes.to_vec()))
}

/// Every mounted path starting with `prefix`.
pub(crate) fn list(prefix: &str) -> Vec<SharedString> {
    let mounts = MOUNTS.read().unwrap_or_else(PoisonError::into_inner);
    mounts
        .iter()
        .flat_map(|(id, files)| {
            files
                .keys()
                .map(move |relative| format!("{ICON_MOUNT_PREFIX}{id}/{relative}"))
        })
        .filter(|path| path.starts_with(prefix))
        .map(SharedString::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svg(text: &str) -> Arc<[u8]> {
        Arc::from(text.as_bytes())
    }

    #[test]
    fn serves_mounted_files_until_the_mount_drops() {
        let mount = IconMount::new([("icons/pod.svg".to_owned(), svg("<svg>pod</svg>"))]);
        let path = mount.asset_path("icons/pod.svg");
        assert!(path.starts_with(ICON_MOUNT_PREFIX));
        assert!(mount.contains("icons/pod.svg"));
        assert_eq!(load(&path).as_deref(), Some(&b"<svg>pod</svg>"[..]));
        assert!(list(ICON_MOUNT_PREFIX).contains(&path));
        assert_eq!(load(&mount.asset_path("icons/other.svg")), None);

        drop(mount);
        assert_eq!(load(&path), None, "a dropped mount serves nothing");
    }

    #[test]
    fn every_mount_gets_its_own_paths() {
        let first = IconMount::new([("a.svg".to_owned(), svg("<svg>1</svg>"))]);
        let second = IconMount::new([("a.svg".to_owned(), svg("<svg>2</svg>"))]);
        assert_ne!(first.asset_path("a.svg"), second.asset_path("a.svg"));
        assert_eq!(
            load(&second.asset_path("a.svg")).as_deref(),
            Some(&b"<svg>2</svg>"[..])
        );
    }

    #[test]
    fn malformed_paths_miss() {
        for path in [
            "icon-themes/",
            "icon-themes/x/a.svg",
            "icon-themes/99999999/a.svg",
            "icons/box.svg",
        ] {
            assert_eq!(load(path), None, "{path}");
        }
    }
}
