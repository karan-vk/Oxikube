//! Reading the output of `ls -la` (and of the `find` fallback) into [`FileEntry`]s.
//!
//! Pure functions, tested against a corpus of real outputs (GNU coreutils, busybox, Alpine,
//! symlinks, names with spaces, unicode, permission denied, empty directories).
//!
//! # What is tolerated
//!
//! * the GNU long format (`-rw-r--r-- 1 root root 12 Mar  5 12:34 name`), with `--full-time`
//!   (`2024-03-05 12:34:56.123456789 +0000`) or an ISO date (`2024-03-05 12:34`);
//! * busybox's, whose columns are padded differently and which may print `ctime`-style dates
//!   (`Tue Mar  5 12:34:56 2024`);
//! * a trailing `.` or `+` on the mode (SELinux, ACLs), device entries (`1, 3` where the size is),
//!   `name -> target` for links, names with spaces and any unicode;
//! * `total N`, `.` and `..`, and the `ls:` complaints on stderr, which are not entries.
//!
//! The container's locale is forced to `C` by the command (`LC_ALL=C`), so month names are
//! English; a line this parser cannot read is skipped, never an error for the whole directory.

mod date;
mod find;

use super::entry::{FileEntry, FileKind};

pub use find::parse_find;

/// The result of reading an `ls` output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedLs {
    /// The entries, in output order.
    pub entries: Vec<FileEntry>,
    /// Entries `ls` listed without details (`d????????? ? ? ? ?  ? name`): the container refused
    /// to read them.
    pub unreadable: usize,
}

/// Reads the stdout of `ls -la [--full-time]`. `now` (Unix seconds) works out the year of dates
/// that have none.
pub fn parse_ls(output: &str, now: i64) -> ParsedLs {
    let mut parsed = ParsedLs::default();
    for line in output.lines() {
        let Some(entry) = parse_line(line, now) else {
            continue;
        };
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        if entry.mode.is_empty() && entry.owner.is_empty() {
            parsed.unreadable += 1;
        }
        parsed.entries.push(entry);
    }
    parsed
}

/// The next whitespace-separated token of `rest`, which then starts after it.
fn token<'a>(rest: &mut &'a str) -> Option<&'a str> {
    let trimmed = rest.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let (word, after) = trimmed.split_at(end);
    *rest = after;
    Some(word)
}

fn kind_of(letter: char) -> Option<FileKind> {
    match letter {
        '-' => Some(FileKind::File),
        'd' => Some(FileKind::Directory),
        'l' => Some(FileKind::Symlink),
        'c' | 'b' | 'p' | 's' => Some(FileKind::Special),
        _ => None,
    }
}

fn parse_line(line: &str, now: i64) -> Option<FileEntry> {
    let mut rest = line;
    let mode_token = token(&mut rest)?;
    let mut letters = mode_token.chars();
    let kind = kind_of(letters.next()?)?;
    // `drwxr-xr-x`, plus an optional `.`, `+` or `@` (SELinux, ACL, extended attributes).
    let permissions: String = letters.collect();
    let permissions = permissions.trim_end_matches(['.', '+', '@']);
    if permissions.len() != 9 {
        return None;
    }
    if permissions.contains('?') {
        return unreadable(kind, rest);
    }
    let _links = token(&mut rest)?;
    let owner = token(&mut rest)?;
    let group = token(&mut rest)?;
    let size_token = token(&mut rest)?;
    let size = if matches!(kind, FileKind::Special) && size_token.ends_with(',') {
        // `1, 3`: the device numbers stand where the size does.
        token(&mut rest)?;
        None
    } else {
        Some(size_token.parse::<u64>().ok()?)
    };
    let modified = date::take(&mut rest, now)?;
    let name_part = rest.strip_prefix(' ').unwrap_or(rest);
    let name_part = name_part.trim_end_matches('\r');
    if name_part.is_empty() {
        return None;
    }
    let (name, link_target) = match kind {
        FileKind::Symlink => match name_part.split_once(" -> ") {
            Some((name, target)) => (name, Some(target.to_owned())),
            None => (name_part, None),
        },
        _ => (name_part, None),
    };
    Some(FileEntry {
        name: name.to_owned(),
        kind,
        size,
        mode: permissions.to_owned(),
        modified,
        owner: owner.to_owned(),
        group: group.to_owned(),
        link_target,
    })
}

/// `d????????? ? ? ? ?            ? name`: GNU `ls` could not stat the entry.
fn unreadable(kind: FileKind, mut rest: &str) -> Option<FileEntry> {
    for _ in 0..5 {
        token(&mut rest)?;
    }
    let name = rest
        .strip_prefix(' ')
        .unwrap_or(rest)
        .trim_end_matches('\r');
    (!name.is_empty()).then(|| FileEntry::named(name, kind))
}

#[cfg(test)]
mod tests;
