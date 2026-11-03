//! Icon paths from a theme file: relative to the icon theme's directory, never out of it.
//!
//! An icon theme is untrusted input (a downloaded file, later an extension), so a path that is
//! absolute, has a drive or `..` component, or is not an SVG is refused before anything is read.
//! [`super::read`] additionally checks the canonical path of every file it opens, so a symlink
//! inside the directory cannot point out of it either.

use std::path::{Component, Path};

/// Why an icon path was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IconPathError {
    /// Empty, or only `.` components.
    #[error("the icon path is empty")]
    Empty,
    /// Starts at a root or a drive (`/x.svg`, `C:\x.svg`, `\\server\x.svg`).
    #[error("icon path {0:?} is absolute; icon paths are relative to the icon theme")]
    Absolute(String),
    /// Has a `..` component.
    #[error("icon path {0:?} leaves the icon theme's directory")]
    ParentDir(String),
    /// Not an `.svg` file, or holds a NUL.
    #[error("icon path {0:?} is not an .svg file")]
    NotSvg(String),
    /// Resolves (through a symlink) to a file outside the icon theme's directory.
    #[error("icon path {0:?} resolves outside the icon theme's directory")]
    Escapes(String),
}

/// The normalised form of `raw` (`./icons\pod.svg` -> `icons/pod.svg`), or why it is refused.
///
/// Pure: reads nothing.
///
/// # Errors
///
/// See [`IconPathError`].
pub fn safe_relative_path(raw: &str) -> Result<String, IconPathError> {
    let raw_owned = || raw.to_owned();
    if raw.contains('\0') {
        return Err(IconPathError::NotSvg(raw_owned()));
    }
    let unified = raw.trim().replace('\\', "/");
    let has_drive = unified.as_bytes().get(1) == Some(&b':');
    if unified.starts_with('/') || has_drive || Path::new(raw.trim()).is_absolute() {
        return Err(IconPathError::Absolute(raw_owned()));
    }
    let mut parts: Vec<&str> = Vec::new();
    for component in Path::new(&unified).components() {
        match component {
            Component::Normal(part) => match part.to_str() {
                Some(part) => parts.push(part),
                None => return Err(IconPathError::NotSvg(raw_owned())),
            },
            Component::CurDir => {}
            Component::ParentDir => return Err(IconPathError::ParentDir(raw_owned())),
            Component::RootDir | Component::Prefix(_) => {
                return Err(IconPathError::Absolute(raw_owned()));
            }
        }
    }
    let Some(file) = parts.last() else {
        return Err(IconPathError::Empty);
    };
    let is_svg = Path::new(file)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg"));
    if !is_svg {
        return Err(IconPathError::NotSvg(raw_owned()));
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_relative_paths() {
        assert_eq!(
            safe_relative_path("./icons/pod.svg").unwrap(),
            "icons/pod.svg"
        );
        assert_eq!(
            safe_relative_path("icons\\pod.SVG").unwrap(),
            "icons/pod.SVG"
        );
        assert_eq!(safe_relative_path("a/./b//c.svg").unwrap(), "a/b/c.svg");
    }

    #[test]
    fn refuses_paths_that_leave_the_directory() {
        for raw in [
            "../pod.svg",
            "icons/../../etc/x.svg",
            "./a/..\\..\\b.svg",
            "a/../b.svg",
        ] {
            assert!(
                matches!(safe_relative_path(raw), Err(IconPathError::ParentDir(_))),
                "{raw}"
            );
        }
    }

    #[test]
    fn refuses_absolute_and_drive_paths() {
        for raw in [
            "/etc/x.svg",
            "\\\\server\\share\\x.svg",
            "C:\\x.svg",
            "c:x.svg",
        ] {
            assert!(
                matches!(safe_relative_path(raw), Err(IconPathError::Absolute(_))),
                "{raw}: {:?}",
                safe_relative_path(raw)
            );
        }
    }

    #[test]
    fn refuses_empty_and_non_svg_paths() {
        assert_eq!(safe_relative_path(""), Err(IconPathError::Empty));
        assert_eq!(safe_relative_path("./"), Err(IconPathError::Empty));
        for raw in ["icons/pod.png", "icons/", "pod", "a\0.svg"] {
            assert!(safe_relative_path(raw).is_err(), "{raw}");
        }
    }
}
