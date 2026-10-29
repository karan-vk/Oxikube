//! Building and unpacking the tar archives of [`FakeTarExec`](super::FakeTarExec).

use std::io::Cursor;

use tar::{Archive, Builder, EntryType, Header};

use super::fs::{MemFs, Node, join, normalize};

/// The archive of `parent/name`: entries start at `name`.
pub(super) fn build(fs: &MemFs, parent: &str, name: &str) -> Vec<u8> {
    let root = join(parent, name);
    let strip = normalize(parent);
    let mut builder = Builder::new(Vec::new());
    for (path, node) in fs.subtree(&root) {
        let relative = path
            .strip_prefix(strip.trim_end_matches('/'))
            .unwrap_or(&path)
            .trim_start_matches('/')
            .to_owned();
        let mut header = Header::new_gnu();
        match node {
            Node::Dir { mode } => {
                header.set_entry_type(EntryType::Directory);
                header.set_size(0);
                header.set_mode(mode);
                header.set_cksum();
                builder
                    .append_data(&mut header, format!("{relative}/"), std::io::empty())
                    .expect("appending to an in-memory archive cannot fail");
            }
            Node::File { data, mode } => {
                header.set_entry_type(EntryType::Regular);
                header.set_size(data.len() as u64);
                header.set_mode(mode);
                header.set_cksum();
                builder
                    .append_data(&mut header, relative, data.as_slice())
                    .expect("appending to an in-memory archive cannot fail");
            }
        }
    }
    builder
        .into_inner()
        .expect("finishing an in-memory archive cannot fail")
}

/// Why an unpack stopped.
pub(super) enum UnpackError {
    /// The bytes are not a tar archive.
    Corrupt,
    /// A member name that is absolute or climbs with `..`.
    UnsafeName(String),
    /// `check` refused writing the path.
    Refused(String, &'static str),
}

/// Unpacks `archive` below `dir`. `check` is asked before each write and may refuse with the
/// text GNU tar would print. Members written before an error stay.
pub(super) fn unpack(
    fs: &mut MemFs,
    dir: &str,
    archive: &[u8],
    check: &dyn Fn(&str) -> Option<&'static str>,
) -> Result<(), UnpackError> {
    let mut archive = Archive::new(Cursor::new(archive));
    let entries = archive.entries().map_err(|_| UnpackError::Corrupt)?;
    for entry in entries {
        let mut entry = entry.map_err(|_| UnpackError::Corrupt)?;
        let name = entry
            .path()
            .map_err(|_| UnpackError::Corrupt)?
            .to_string_lossy()
            .into_owned();
        if name.starts_with('/') || name.split('/').any(|part| part == "..") {
            return Err(UnpackError::UnsafeName(name));
        }
        let target = join(dir, &name);
        let kind = entry.header().entry_type();
        if !matches!(kind, EntryType::Regular | EntryType::Directory) {
            continue;
        }
        if let Some(reason) = check(&target) {
            return Err(UnpackError::Refused(target, reason));
        }
        let mode = entry.header().mode().unwrap_or(0o644);
        if kind == EntryType::Directory {
            fs.put_dir(&target, mode);
        } else {
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut data).map_err(|_| UnpackError::Corrupt)?;
            fs.put_file(&target, data, mode);
        }
    }
    Ok(())
}
