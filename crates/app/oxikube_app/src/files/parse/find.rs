//! Reading the output of the `find` fallback: `d /dir/name`, `l /dir/name`, `f /dir/name`.

use crate::files::entry::{FileEntry, FileKind, directory_argument};

/// Reads the lines the fallback script prints for the directory `dir`: a kind letter, a space and
/// the entry's path. Lines that are not in that shape are skipped.
pub fn parse_find(output: &str, dir: &str) -> Vec<FileEntry> {
    let prefix = directory_argument(dir);
    output
        .lines()
        .filter_map(|line| {
            let (letter, path) = line.split_once(' ')?;
            let kind = match letter {
                "d" => FileKind::Directory,
                "l" => FileKind::Symlink,
                "f" => FileKind::File,
                _ => return None,
            };
            // `find` prints the directory as it was given; the name is what follows it.
            let name = path
                .strip_prefix(&prefix)
                .or_else(|| path.strip_prefix(dir.trim_end_matches('/')))
                .unwrap_or(path)
                .trim_start_matches('/');
            (!name.is_empty() && !name.contains('/')).then(|| FileEntry::named(name, kind))
        })
        .collect()
}
