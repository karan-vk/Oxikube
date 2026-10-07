//! Lint: every text size in a UI crate goes through the zoom helper [`oxikube_ui::u`] (E05-U557).
//!
//! `ui_scale` multiplies every pixel size, so a font size that skips [`u`] stays at 100 % while
//! its row grows (the sidebar, the namespace selector and the overview tiles did exactly that). The
//! lint scans the sources of `crates/ui/*` and `bins/*` and rejects
//!
//! - `.text_size(<arg>)` whose argument does not start with `u(`, such as
//!   `.text_size(tokens.font.body)`;
//! - rem-based shorthands (`.text_xs()`, `.text_sm()`, `.text_lg()` ...), which ignore the zoom.
//!
//! A deliberate exception (a size the user sets in absolute points, such as the terminal font)
//! carries `// ui-scale: exempt` on the same line or the line above, with the reason.

use std::path::{Path, PathBuf};

const EXEMPT: &str = "ui-scale: exempt";
const SHORTHANDS: [&str; 10] = [
    ".text_xs(",
    ".text_sm(",
    ".text_base(",
    ".text_lg(",
    ".text_xl(",
    ".text_2xl(",
    ".text_3xl(",
    ".text_4xl(",
    ".text_5xl(",
    ".text_6xl(",
];

/// The violations in one source file, as `line: message`.
fn violations(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let exempt = line.contains(EXEMPT) || (index > 0 && lines[index - 1].contains(EXEMPT));
        if exempt {
            continue;
        }
        if let Some(shorthand) = SHORTHANDS.iter().find(|s| code.contains(**s)) {
            found.push(format!(
                "{}: `{}…)` ignores ui_scale; use `.text_size(u(..))`",
                index + 1,
                shorthand
            ));
        }
        let Some(at) = code.find(".text_size(") else {
            continue;
        };
        // The argument may start on the next line when rustfmt breaks the call.
        let mut rest = code[at + ".text_size(".len()..].trim_start().to_owned();
        if rest.is_empty() {
            rest = lines
                .get(index + 1)
                .map_or("", |l| l.trim_start())
                .to_owned();
        }
        if !rest.starts_with("u(") {
            found.push(format!(
                "{}: `.text_size({}` does not go through `u(..)`",
                index + 1,
                rest.chars().take(40).collect::<String>()
            ));
        }
    }
    found
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn ui_sources_scale_every_text_size() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut files = Vec::new();
    for group in ["crates/ui", "bins"] {
        let Ok(entries) = std::fs::read_dir(root.join(group)) else {
            continue;
        };
        for entry in entries.flatten() {
            rust_files(&entry.path().join("src"), &mut files);
        }
    }
    assert!(files.len() > 50, "scanned only {} files", files.len());
    let mut report = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file).expect("readable source");
        for message in violations(&source) {
            report.push(format!("{}:{message}", file.display()));
        }
    }
    assert!(
        report.is_empty(),
        "text sizes that skip the ui_scale helper `u(..)`:\n{}",
        report.join("\n")
    );
}

#[test]
fn the_lint_rejects_bare_token_sizes() {
    let bad = "div()\n    .text_size(tokens.font.body)\n    .child(x)";
    assert_eq!(violations(bad).len(), 1, "{:?}", violations(bad));
    let split = "div().text_size(\n    tokens.font.small,\n)";
    assert_eq!(violations(split).len(), 1);
    assert_eq!(violations("div().text_sm()").len(), 1);
    assert_eq!(violations("div().text_xs().text_lg()").len(), 1);
}

#[test]
fn the_lint_accepts_scaled_and_exempt_sizes() {
    assert!(violations("div().text_size(u(tokens.font.body))").is_empty());
    assert!(violations("div().text_size(u(px(12.)))").is_empty());
    assert!(violations("div().text_size(\n    u(tokens.font.small),\n)").is_empty());
    assert!(violations("// text_size(tokens.font.body) in a comment").is_empty());
    assert!(
        violations("// ui-scale: exempt (absolute points)\ndiv().text_size(px(13.))").is_empty()
    );
    assert!(violations("div().text_size(px(13.)) // ui-scale: exempt").is_empty());
}
