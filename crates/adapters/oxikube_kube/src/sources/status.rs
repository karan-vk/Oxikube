//! How the last read of each source went: the port's [`SourceStatus`] rows (E06-S05).
//!
//! Built from the loader's per-file results ([`SourceInfo`]) and the diagnostics of the layout,
//! so the sources screen can say "found, 3 contexts" or "not a valid kubeconfig" next to each
//! row. Messages are fixed wording that names files, never their content: the parser's own
//! text can quote the offending line, which may be a token.

use std::collections::HashMap;
use std::path::Path;

use oxikube_ports::cluster_source::{SourceId, SourceKind, SourceState, SourceStatus};

use super::layout::{Layout, SourceEntry};
use crate::kubeconfig::{Diagnostic, SourceInfo, SourceStatus as FileStatus};

/// How many broken files of a directory are named in its message.
const NAMED_FILES: usize = 3;

/// One status per configured source, in source order.
pub(super) fn build(
    layout: &Layout,
    files: &[SourceInfo],
    diagnostics: &[Diagnostic],
    owned: &HashMap<SourceId, usize>,
) -> Vec<SourceStatus> {
    layout
        .entries
        .iter()
        .map(|entry| {
            let contexts = owned.get(&entry.source.id).copied().unwrap_or(0);
            let (state, message) = if entry.source.kind == SourceKind::KubeconfigDir {
                dir_state(entry, files, diagnostics, &layout.diagnostics)
            } else {
                file_state(entry, files, diagnostics)
            };
            SourceStatus {
                source: entry.source.clone(),
                state,
                contexts,
                message,
            }
        })
        .collect()
}

fn info<'a>(files: &'a [SourceInfo], path: &Path) -> Option<&'a SourceInfo> {
    files.iter().find(|info| info.path == path)
}

/// The state of a source made of one file (or of one pasted text, or the in-cluster account).
fn file_state(
    entry: &SourceEntry,
    files: &[SourceInfo],
    diagnostics: &[Diagnostic],
) -> (SourceState, Option<String>) {
    let Some(path) = entry.files.first() else {
        return (SourceState::Found, None);
    };
    match info(files, path) {
        // Another source lists the same file and was read first.
        None => (
            SourceState::Found,
            Some("Same file as an earlier source".into()),
        ),
        Some(info) => describe(info.status, path, diagnostics),
    }
}

/// The state and message for a file of status `status` at `path`.
fn describe(
    status: FileStatus,
    path: &Path,
    diagnostics: &[Diagnostic],
) -> (SourceState, Option<String>) {
    match status {
        FileStatus::Loaded => (SourceState::Found, None),
        FileStatus::Missing => (SourceState::Missing, Some("File not found".into())),
        FileStatus::Blank => (SourceState::Blank, Some("File is empty".into())),
        FileStatus::Unreadable => {
            let reason = diagnostics.iter().find_map(|d| match d {
                Diagnostic::Unreadable { path: p, reason } if p == path => Some(reason.clone()),
                _ => None,
            });
            let message = match reason {
                Some(reason) => format!("Could not be read ({reason})"),
                None => "Could not be read".to_owned(),
            };
            (SourceState::Unreadable, Some(message))
        }
        FileStatus::Incompatible => {
            let reason = diagnostics.iter().find_map(|d| match d {
                Diagnostic::Incompatible { path: p, reason } if p == path => Some(reason.clone()),
                _ => None,
            });
            let message = match reason {
                Some(reason) => format!("Cannot be merged with the other kubeconfigs ({reason})"),
                None => "Cannot be merged with the other kubeconfigs".to_owned(),
            };
            (SourceState::Invalid, Some(message))
        }
        // `Unparsable`, and any status a later loader version adds.
        _ => (SourceState::Invalid, Some("Not a valid kubeconfig".into())),
    }
}

/// The state of a directory: the directory itself, then its files.
fn dir_state(
    entry: &SourceEntry,
    files: &[SourceInfo],
    diagnostics: &[Diagnostic],
    layout_diagnostics: &[Diagnostic],
) -> (SourceState, Option<String>) {
    let dir = entry.source.path.as_deref();
    for diagnostic in layout_diagnostics {
        match diagnostic {
            Diagnostic::MissingFile { path } if Some(path.as_path()) == dir => {
                return (SourceState::Missing, Some("Folder not found".into()));
            }
            Diagnostic::Unreadable { path, reason } if Some(path.as_path()) == dir => {
                return (
                    SourceState::Unreadable,
                    Some(format!("Folder could not be read ({reason})")),
                );
            }
            _ => {}
        }
    }
    if entry.files.is_empty() {
        return (
            SourceState::Blank,
            Some("The folder has no kubeconfig files".into()),
        );
    }
    let broken: Vec<String> = entry
        .files
        .iter()
        .filter_map(|path| {
            let info = info(files, path)?;
            if info.status == FileStatus::Loaded {
                return None;
            }
            let (_, message) = describe(info.status, path, diagnostics);
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some(format!(
                "{name}: {}",
                message.unwrap_or_default().to_lowercase()
            ))
        })
        .collect();
    if broken.is_empty() {
        return (SourceState::Found, None);
    }
    let mut message = format!(
        "{} of {} files skipped: {}",
        broken.len(),
        entry.files.len(),
        broken
            .iter()
            .take(NAMED_FILES)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    );
    if broken.len() > NAMED_FILES {
        message.push_str("; ...");
    }
    // Only when every file failed is the folder itself unusable; otherwise the rest loaded.
    let state = if broken.len() == entry.files.len() {
        SourceState::Invalid
    } else {
        SourceState::Found
    };
    (state, Some(message))
}
