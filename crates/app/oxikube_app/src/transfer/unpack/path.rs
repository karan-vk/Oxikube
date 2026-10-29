//! Which archive paths and link targets may land under the destination.

use std::path::{Component, Path, PathBuf};

/// Why an entry's path cannot be unpacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Refusal {
    /// It starts at the filesystem root (`/etc/passwd`).
    Absolute,
    /// It climbs out with `..`.
    ParentDir,
    /// It is not UTF-8, or holds a character a path component cannot safely carry here (`\`,
    /// a drive letter on Windows).
    Unrepresentable,
}

impl Refusal {
    pub(super) fn reason(self) -> &'static str {
        match self {
            Refusal::Absolute => "it is an absolute path",
            Refusal::ParentDir => "it contains `..`",
            Refusal::Unrepresentable => "it is not a portable relative path",
        }
    }
}

/// The components of a relative archive path, `.` and empty ones dropped.
fn components(raw: &[u8]) -> Result<Vec<&str>, Refusal> {
    let text = std::str::from_utf8(raw).map_err(|_| Refusal::Unrepresentable)?;
    if text.starts_with('/') {
        return Err(Refusal::Absolute);
    }
    let mut parts = Vec::new();
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(Refusal::ParentDir),
            part if part.contains('\\') || part.contains('\0') => {
                return Err(Refusal::Unrepresentable);
            }
            part if cfg!(windows) && part.contains(':') => return Err(Refusal::Unrepresentable),
            part => parts.push(part),
        }
    }
    Ok(parts)
}

/// Where an entry named `raw` goes, relative to the destination: `Ok(None)` for the archive's
/// root (`./`), which is the destination itself.
pub(super) fn entry_path(raw: &[u8]) -> Result<Option<PathBuf>, Refusal> {
    let parts = components(raw)?;
    if parts.is_empty() {
        return Ok(None);
    }
    let path: PathBuf = parts.iter().collect();
    // Belt and braces: whatever the platform parses, only plain names may remain.
    if !path
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err(Refusal::Unrepresentable);
    }
    Ok(Some(path))
}

/// Whether a link at `entry` pointing to `target` resolves inside the destination: relative, and
/// never climbing above it.
pub(super) fn link_stays_inside(entry: &Path, target: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(target) else {
        return false;
    };
    if text.is_empty() || text.starts_with('/') {
        return false;
    }
    let mut depth = entry.components().count().saturating_sub(1);
    for part in text.split('/') {
        match part {
            "" | "." => {}
            ".." => match depth.checked_sub(1) {
                Some(up) => depth = up,
                None => return false,
            },
            part if part.contains('\\') => return false,
            _ => depth += 1,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_relative_paths_are_kept() {
        assert_eq!(
            entry_path(b"log/app.log"),
            Ok(Some(PathBuf::from("log/app.log")))
        );
        assert_eq!(entry_path(b"./log//./a"), Ok(Some(PathBuf::from("log/a"))));
        assert_eq!(entry_path(b"./"), Ok(None));
        assert_eq!(entry_path(b"dir/"), Ok(Some(PathBuf::from("dir"))));
    }

    #[test]
    fn escaping_paths_are_refused() {
        assert_eq!(entry_path(b"/etc/passwd"), Err(Refusal::Absolute));
        assert_eq!(entry_path(b"../x"), Err(Refusal::ParentDir));
        assert_eq!(entry_path(b"a/../../x"), Err(Refusal::ParentDir));
        assert_eq!(entry_path(b"a/..\\..\\x"), Err(Refusal::Unrepresentable));
        assert_eq!(entry_path(b"\xff\xfe"), Err(Refusal::Unrepresentable));
    }

    #[test]
    fn links_must_resolve_inside() {
        let entry = Path::new("a/b/link");
        assert!(link_stays_inside(entry, b"target"));
        assert!(link_stays_inside(entry, b"../c"));
        assert!(link_stays_inside(entry, b"../../c"));
        assert!(!link_stays_inside(entry, b"../../../c"));
        assert!(!link_stays_inside(entry, b"/etc/passwd"));
        assert!(!link_stays_inside(Path::new("link"), b".."));
        assert!(!link_stays_inside(entry, b""));
        assert!(link_stays_inside(Path::new("link"), b"./x/../y"));
    }
}
