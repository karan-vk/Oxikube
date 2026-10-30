//! Loading manifests from a file, a directory or text (E20-S01): the app half of
//! `oxikube_domain::manifests`, which only parses text.
//!
//! [`load_path`] walks a directory over [`FsPort`] (which only lists one level and reads whole
//! files), so the walk lives here:
//!
//! * `.yaml`, `.yml` and `.json` files (any case) are read; everything else is ignored.
//! * Entries are visited depth first in lexicographic order of their names, so the result is the
//!   same on every run and platform.
//! * Hidden entries (`.git`, `.DS_Store`) are skipped. Symbolic links are never followed, which
//!   rules out loops; each is reported as a [`LoadNote`].
//! * `kustomization.yaml` / `.yml` are skipped with a note: that directory is applied through
//!   kustomize.
//! * [`LoadLimits`] keep a mistaken path (`/`) from freezing the app: a file over the size limit
//!   is one [`ManifestError`]; too many files, too many bytes in all, or a tree deeper than the
//!   limit stop the load with an error.
//!
//! Errors name the file and line, never document text (the parser drops it). Each file is parsed
//! on the runtime's blocking pool ([`off_io_workers`]), never on an I/O worker.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxikube_domain::manifests::{ManifestDoc, ManifestError, SourceId, parse_documents};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{DirEntry, EntryKind, FsPort};

use crate::offload::off_io_workers;

/// Bounds on one load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadLimits {
    /// Largest single file read, in bytes.
    pub max_file_bytes: u64,
    /// Most manifest files in one directory tree.
    pub max_files: usize,
    /// Most bytes read in all.
    pub max_total_bytes: u64,
    /// Deepest directory level below the given one that is entered.
    pub max_depth: usize,
}

impl Default for LoadLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 10 * 1024 * 1024,
            max_files: 1000,
            max_total_bytes: 64 * 1024 * 1024,
            max_depth: 16,
        }
    }
}

/// Something the user should know that is not an error in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadNote {
    /// The file or directory concerned.
    pub path: PathBuf,
    /// What happened to it.
    pub message: String,
}

/// Everything one load found.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadedManifests {
    /// The objects, in file order then document order.
    pub docs: Vec<ManifestDoc>,
    /// Documents and files that could not be read.
    pub errors: Vec<ManifestError>,
    /// Entries skipped on purpose.
    pub notes: Vec<LoadNote>,
    /// Manifest files read.
    pub files: usize,
}

/// Reads the objects in `text`, reported under `label`. The same parser as for a file.
pub fn load_text(text: &str, label: &str) -> LoadedManifests {
    let (docs, errors) = parse_documents(text, SourceId::new(label));
    LoadedManifests {
        docs,
        errors,
        ..LoadedManifests::default()
    }
}

/// Whether `path` names a manifest file by its extension.
fn is_manifest_file(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["yaml", "yml", "json"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    })
}

fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
}

fn is_kustomization(path: &Path) -> bool {
    matches!(
        file_name(path),
        "kustomization.yaml" | "kustomization.yml" | "Kustomization"
    )
}

/// Reads the manifests at `path`: a file (any extension, since the user named it) or a directory
/// (recursively, see the module docs).
///
/// # Errors
///
/// `NotFound` for a missing path; `Validation` when a limit in `limits` stops the load; the
/// port's error for a directory that cannot be listed or read.
pub async fn load_path(
    fs: &Arc<dyn FsPort>,
    path: &Path,
    limits: LoadLimits,
) -> OxiResult<LoadedManifests> {
    let mut loaded = LoadedManifests::default();
    let mut total_bytes = 0_u64;
    match fs.list(path).await {
        Ok(entries) => {
            walk(fs, path, entries, limits, &mut loaded, &mut total_bytes).await?;
        }
        // A file where a directory is expected, or nothing: `read` tells which.
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::Validation) => {
            let size = known_size(fs, path).await;
            read_file(fs, path, size, limits, &mut loaded, &mut total_bytes).await?;
        }
        Err(error) => return Err(error),
    }
    Ok(loaded)
}

/// The size of the file at `path`, from its parent's listing, when the port can say.
async fn known_size(fs: &Arc<dyn FsPort>, path: &Path) -> Option<u64> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty())?;
    let entries = fs.list(parent).await.ok()?;
    entries.into_iter().find(|e| e.path == path)?.size
}

/// Depth-first, name-ordered walk of the tree whose first level is `entries`.
async fn walk(
    fs: &Arc<dyn FsPort>,
    root: &Path,
    entries: Vec<DirEntry>,
    limits: LoadLimits,
    loaded: &mut LoadedManifests,
    total_bytes: &mut u64,
) -> OxiResult<()> {
    // A stack of entries, children pushed in reverse so the smallest name pops first.
    let mut stack: Vec<(DirEntry, usize)> =
        sorted(entries).into_iter().rev().map(|e| (e, 1)).collect();
    while let Some((entry, depth)) = stack.pop() {
        let name = file_name(&entry.path);
        if name.starts_with('.') {
            continue;
        }
        match entry.kind {
            EntryKind::Symlink => loaded.notes.push(LoadNote {
                path: entry.path,
                message: "symbolic link not followed".into(),
            }),
            EntryKind::Dir if depth > limits.max_depth => loaded.notes.push(LoadNote {
                path: entry.path,
                message: format!("deeper than {} levels, skipped", limits.max_depth),
            }),
            EntryKind::Dir => {
                let children = fs.list(&entry.path).await?;
                stack.extend(sorted(children).into_iter().rev().map(|e| (e, depth + 1)));
            }
            EntryKind::File if is_kustomization(&entry.path) => loaded.notes.push(LoadNote {
                message: "kustomization found; apply this directory through kustomize".into(),
                path: entry.path,
            }),
            EntryKind::File if is_manifest_file(&entry.path) => {
                if loaded.files >= limits.max_files {
                    return Err(OxiError::validation(format!(
                        "{} holds more than {} manifest files; choose a smaller directory",
                        root.display(),
                        limits.max_files
                    )));
                }
                read_file(fs, &entry.path, entry.size, limits, loaded, total_bytes).await?;
            }
            EntryKind::File => {}
        }
    }
    Ok(())
}

fn sorted(mut entries: Vec<DirEntry>) -> Vec<DirEntry> {
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

/// Reads and parses one file into `loaded`. A file that is too big or not UTF-8 is an error entry;
/// the total-size limit and a failing read stop the load.
async fn read_file(
    fs: &Arc<dyn FsPort>,
    path: &Path,
    size: Option<u64>,
    limits: LoadLimits,
    loaded: &mut LoadedManifests,
    total_bytes: &mut u64,
) -> OxiResult<()> {
    let source = SourceId::file(path);
    let file_error = |message: String| ManifestError {
        source: source.clone(),
        line: 1,
        column: None,
        message,
    };
    if let Some(size) = size {
        if size > limits.max_file_bytes {
            loaded.errors.push(file_error(too_big_message(limits)));
            return Ok(());
        }
        if total_bytes.saturating_add(size) > limits.max_total_bytes {
            return Err(total_error(limits));
        }
    }
    let bytes = fs.read(path).await?;
    let len = bytes.len() as u64;
    if len > limits.max_file_bytes {
        loaded.errors.push(file_error(too_big_message(limits)));
        return Ok(());
    }
    *total_bytes = total_bytes.saturating_add(len);
    if *total_bytes > limits.max_total_bytes {
        return Err(total_error(limits));
    }
    loaded.files += 1;
    let Ok(text) = String::from_utf8(bytes) else {
        loaded
            .errors
            .push(file_error("the file is not valid UTF-8".into()));
        return Ok(());
    };
    // Parsing megabytes of YAML is CPU work: not on an I/O worker.
    let (docs, errors) = off_io_workers(move || parse_documents(&text, source)).await?;
    loaded.docs.extend(docs);
    loaded.errors.extend(errors);
    Ok(())
}

fn too_big_message(limits: LoadLimits) -> String {
    format!(
        "the file is larger than the {} MiB limit",
        limits.max_file_bytes / (1024 * 1024)
    )
}

fn total_error(limits: LoadLimits) -> OxiError {
    OxiError::validation(format!(
        "more than {} MiB of manifests; choose a smaller directory",
        limits.max_total_bytes / (1024 * 1024)
    ))
}
