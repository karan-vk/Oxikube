//! The file browser's data: [`FileEntry`] and the [`Listing`] of a directory.

/// What an entry of a directory listing is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// A symbolic link. It may point to a directory: opening it lists the target.
    Symlink,
    /// A device, socket or pipe.
    Special,
}

/// One entry of a directory in a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// The name, without its directory.
    pub name: String,
    /// What it is.
    pub kind: FileKind,
    /// The size in bytes. `None` for a device, and for a listing that could not say.
    pub size: Option<u64>,
    /// The permission bits as `ls` prints them without the type letter (`rwxr-xr-x`, `rwsr-xr-x`
    /// for set-uid). Empty when the listing could not say (the `find` fallback, an unreadable
    /// entry).
    pub mode: String,
    /// The modification time in seconds since the Unix epoch, UTC. `ls` without `--full-time`
    /// has no year for recent files and no time zone: the year is worked out from the clock and
    /// the zone is taken to be UTC.
    pub modified: Option<i64>,
    /// The owner as `ls` names it.
    pub owner: String,
    /// The group as `ls` names it.
    pub group: String,
    /// Where a symbolic link points.
    pub link_target: Option<String>,
}

impl FileEntry {
    /// An entry known by name and kind only (the `find` fallback).
    pub fn named(name: impl Into<String>, kind: FileKind) -> Self {
        Self {
            name: name.into(),
            kind,
            size: None,
            mode: String::new(),
            modified: None,
            owner: String::new(),
            group: String::new(),
            link_target: None,
        }
    }

    /// A dot file.
    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }

    /// Whether opening it may list a directory: a directory, or a link (which may point to one).
    pub fn is_navigable(&self) -> bool {
        matches!(self.kind, FileKind::Directory | FileKind::Symlink)
    }
}

/// How a [`Listing`] was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingSource {
    /// `ls -la` (with `--full-time` where the container's `ls` has it).
    Ls,
    /// The container has no `ls`: `find` gave names and kinds only.
    Find,
}

/// The entries of one directory of a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    /// The directory, as asked.
    pub path: String,
    /// Its entries, without `.` and `..`, in the order the program printed them.
    pub entries: Vec<FileEntry>,
    /// How many entries the container refused to describe (permission denied while reading
    /// their details); they are listed by name with no details.
    pub unreadable: usize,
    /// How the listing was made.
    pub source: ListingSource,
}

/// `path` with one trailing slash, so `ls` lists the contents of a link to a directory instead of
/// the link itself.
pub fn directory_argument(path: &str) -> String {
    if path.ends_with('/') {
        path.to_owned()
    } else {
        format!("{path}/")
    }
}

/// The directory above `path`; `/` is its own parent.
pub fn parent_of(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_owned(),
        Some(at) => trimmed[..at].to_owned(),
    }
}

/// `name` inside the directory `dir`.
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}
