//! `docs/PERFORMANCE.md` tells contributors where the perf harness lives: every repository path it
//! cites in backticks must exist, so a moved module cannot leave the reproduction steps pointing
//! at nothing.

use std::path::Path;

/// The backticked spans of `text` that look like a repository path.
fn cited_paths(text: &str) -> Vec<&str> {
    const ROOTS: [&str; 4] = ["bins/", "crates/", "xtask/", "docs/"];
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| ROOTS.iter().any(|root| span.starts_with(root)))
        .filter(|span| !span.contains(char::is_whitespace) && !span.contains(['<', '*']))
        .collect()
}

#[test]
fn every_path_the_performance_doc_cites_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let doc = std::fs::read_to_string(root.join("docs/PERFORMANCE.md")).expect("read the doc");
    let cited = cited_paths(&doc);
    assert!(
        cited.contains(&"bins/oxikube/src/perf_table/mod.rs"),
        "the doc names the --perf-table driver"
    );
    let missing: Vec<_> = cited
        .into_iter()
        .filter(|path| !root.join(path).exists())
        .collect();
    assert!(
        missing.is_empty(),
        "docs/PERFORMANCE.md cites paths that do not exist: {missing:?}"
    );
}

#[test]
fn cited_paths_are_the_backticked_repository_paths() {
    let text = "see `bins/a.rs` and `cargo xtask perf`, `crates/b/<name>.rs`, `xtask/src/c.rs`";
    assert_eq!(cited_paths(text), ["bins/a.rs", "xtask/src/c.rs"]);
}
