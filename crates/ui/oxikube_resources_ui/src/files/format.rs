//! The text of a file listing's cells: sizes, modes and times.

use jiff::Timestamp;
use oxikube_app::files::{FileEntry, FileKind};

/// `1536` as `1.5 KiB`; `12` as `12 B`.
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["KiB", "MiB", "GiB", "TiB", "PiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "a size shown to one decimal does not need the low bits"
    )]
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `1709642040` as `2024-03-05 12:34` (UTC: the container's zone is not known).
pub fn time_text(seconds: i64) -> String {
    Timestamp::from_second(seconds)
        .map(|at| at.strftime("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// The type letter and permission bits as `ls` shows them: `drwxr-xr-x`. Empty when the listing
/// gave no details.
pub fn mode_text(entry: &FileEntry) -> String {
    if entry.mode.is_empty() {
        return String::new();
    }
    let letter = match entry.kind {
        FileKind::File => '-',
        FileKind::Directory => 'd',
        FileKind::Symlink => 'l',
        FileKind::Special => 's',
    };
    format!("{letter}{}", entry.mode)
}

/// The size column: empty for a directory (its size is the directory file's, not its content's)
/// and for entries with none.
pub fn entry_size_text(entry: &FileEntry) -> String {
    match (entry.kind, entry.size) {
        (FileKind::File | FileKind::Symlink, Some(size)) => size_text(size),
        _ => String::new(),
    }
}

/// `link -> target` for a link, the name otherwise.
pub fn name_text(entry: &FileEntry) -> String {
    match &entry.link_target {
        Some(target) => format!("{} -> {target}", entry.name),
        None => entry.name.clone(),
    }
}
