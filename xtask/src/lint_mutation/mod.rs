//! Mutation-door lint (E19-S03, ADR 0012).
//!
//! Writes to a cluster are reachable only through `oxikube_app::guard::Mutation`, which the
//! guard hands to the handler of an admitted command. The compile-time barrier is the
//! `ResourceReader` / `ResourceWriter` split plus the crate-private `Mutation` constructor; this
//! lint closes the rest: no non-test source of a `ui`, `platform` or `bins` crate may
//!
//! 1. name `Mutation` (it is the app layer's capability token),
//! 2. name `ResourceWriter` other than in `impl ResourceWriter for T` (implementing a fake world
//!    is fine; holding or calling a writer is not), or
//! 3. call a mutating method (`create`, `replace`, `patch`, `delete`, `delete_collection`,
//!    `scale`, `evict`, `create_subresource`, `patch_subresource`) on a handle typed
//!    `ResourceWriter` / `ResourcePort` or obtained from a `.writer()` call.
//!
//! The scan parses sources with `syn`, so strings, comments and `#[cfg(test)]` items are ignored.
//! Dev-dependency targets (tests, benches, examples) are never visited: only the crate's lib and
//! bin module trees are. There is no allow-list; a violation is fixed by routing the action
//! through a `Command`.

mod modules;
mod scan;
#[cfg(test)]
mod tests;

use cargo_metadata::Metadata;

use crate::lint_deps::Layer;

use scan::scan_source;

/// Scan every `ui`, `platform` and `bins` crate of the workspace; one message per violation.
pub fn check(meta: &Metadata) -> Vec<String> {
    let root = meta.workspace_root.as_std_path();
    let mut errors = Vec::new();
    for pkg in meta.workspace_packages() {
        if !matches!(
            Layer::of(pkg, root),
            Some(Layer::Ui | Layer::Platform | Layer::Bins)
        ) {
            continue;
        }
        for target in pkg.targets.iter().filter(|t| {
            t.kind
                .iter()
                .any(|k| matches!(k.to_string().as_str(), "lib" | "bin"))
        }) {
            for file in modules::module_files(target.src_path.as_std_path()) {
                let shown = file
                    .strip_prefix(root)
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                match std::fs::read_to_string(&file) {
                    Ok(src) => errors.extend(
                        scan_source(&src)
                            .into_iter()
                            .map(|v| format!("{} ({shown}): {v}", pkg.name)),
                    ),
                    Err(e) => errors.push(format!("{} ({shown}): unreadable: {e}", pkg.name)),
                }
            }
        }
    }
    errors
}
