//! Resolve a crate's non-test module tree from its root file.

use std::path::{Path, PathBuf};

use syn::{Item, ItemMod};

/// The crate root and every file reachable through non-`#[cfg(test)]` `mod x;` declarations.
/// Files that are missing or fail to parse are skipped here; the scanner reports parse errors
/// for the files it is given.
pub fn module_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let dir = root.parent().map(Path::to_path_buf).unwrap_or_default();
    visit_file(root, &dir, &mut out);
    out
}

fn visit_file(file: &Path, child_dir: &Path, out: &mut Vec<PathBuf>) {
    out.push(file.to_path_buf());
    let Ok(src) = std::fs::read_to_string(file) else {
        return;
    };
    let Ok(parsed) = syn::parse_file(&src) else {
        return;
    };
    visit_items(&parsed.items, child_dir, out);
}

fn visit_items(items: &[Item], dir: &Path, out: &mut Vec<PathBuf>) {
    for item in items {
        let Item::Mod(module) = item else { continue };
        if super::scan::is_cfg_test(&module.attrs) {
            continue;
        }
        visit_mod(module, dir, out);
    }
}

fn visit_mod(module: &ItemMod, dir: &Path, out: &mut Vec<PathBuf>) {
    let name = module.ident.to_string();
    if let Some((_, items)) = &module.content {
        visit_items(items, &dir.join(&name), out);
        return;
    }
    let flat = dir.join(format!("{name}.rs"));
    let nested = dir.join(&name).join("mod.rs");
    if flat.is_file() {
        visit_file(&flat, &dir.join(&name), out);
    } else if nested.is_file() {
        visit_file(&nested, &dir.join(&name), out);
    }
}
