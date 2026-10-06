//! The names and paths of kubeconfigs Oxikube stores itself.

use std::path::{Component, Path};

use oxikube_domain::{OxiError, OxiResult};

/// The longest name accepted for a pasted kubeconfig (before `.yaml`).
const MAX_NAME_LEN: usize = 64;

/// Names Windows reserves for devices, whatever the extension.
const RESERVED: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// The file name `<name>.yaml` for a pasted kubeconfig called `name`.
///
/// A trailing `.yaml` or `.yml` the user typed is dropped, so `prod.yaml` and `prod` mean the
/// same. Only letters, digits, `-`, `_` and `.` are accepted, and the name may not start with
/// a dot (hidden files are skipped by folder sources), so no separator, `..` or drive prefix
/// can reach the file name: a name never escapes the kubeconfigs directory.
///
/// # Errors
///
/// `Validation`, with a message that says what to change.
pub fn pasted_file_name(name: &str) -> OxiResult<String> {
    let name = name.trim();
    let lower = name.to_ascii_lowercase();
    let stem = ["yaml", "yml"]
        .iter()
        .find_map(|ext| {
            lower
                .strip_suffix(&format!(".{ext}"))
                .map(|rest| &name[..rest.len()])
        })
        .unwrap_or(name);
    if stem.is_empty() {
        return Err(OxiError::validation("Give the kubeconfig a name."));
    }
    if stem.chars().count() > MAX_NAME_LEN {
        return Err(OxiError::validation(format!(
            "The name is too long: use at most {MAX_NAME_LEN} characters."
        )));
    }
    if let Some(bad) = stem
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Err(OxiError::validation(format!(
            "The name cannot contain {bad:?}: use letters, digits, '-', '_' and '.'."
        )));
    }
    if stem.starts_with('.') {
        return Err(OxiError::validation("The name cannot start with a dot."));
    }
    let first = stem.split('.').next().unwrap_or(stem).to_ascii_lowercase();
    if RESERVED.contains(&first.as_str()) {
        return Err(OxiError::validation(format!(
            "{stem:?} is a reserved name: pick another."
        )));
    }
    Ok(format!("{stem}.yaml"))
}

/// Whether `path` is a file directly inside `dir` that Oxikube's own naming could have created:
/// under `dir`, with no `..` or other way out. Lexical (no file access), so a path that merely
/// starts with `dir` but climbs out of it (`<dir>/../x`) is not "ours".
pub fn is_stored_in(dir: &Path, path: &Path) -> bool {
    let Ok(rest) = path.strip_prefix(dir) else {
        return false;
    };
    let mut parts = rest.components();
    matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(_)), None)
    )
}
