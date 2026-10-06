//! The test image list (`fixtures/test-images.txt`) is complete: every container image an
//! integration test or a cluster fixture runs is in it, so `cargo xtask kind-up` has pulled it
//! into the nodes before the first test starts (E04-B01). A test that wrote `"alpine:3"` would
//! pass on a developer's long-lived cluster, where something once pulled it, and time out on a
//! fresh CI cluster; this fails the commit instead.

use std::path::{Path, PathBuf};

use oxikube_testkit::images::{self, is_image_reference};

/// Images that are unpullable on purpose, to exercise pull failures.
fn is_deliberately_unpullable(image: &str) -> bool {
    image.contains(".invalid/") || image.contains("does-not-exist")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Every `.rs` under a `tests/` directory of a crate below `crates/`, and every cluster fixture
/// manifest.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            sources(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn integration_sources() -> Vec<PathBuf> {
    let crates = workspace_root().join("crates");
    let mut all = Vec::new();
    sources(&crates, &mut all);
    all.into_iter()
        .filter(|p| {
            // `crates/<layer>/<crate>/tests/**.rs`: the crates' integration test directories,
            // not the unit tests under `src/`, which never reach a cluster.
            let relative = p.strip_prefix(&crates).unwrap_or(p);
            let parts: Vec<_> = relative.components().map(|c| c.as_os_str()).collect();
            // Only the layers whose tests can reach a cluster; domain tests name images as data.
            let clustered = parts
                .first()
                .is_some_and(|l| ["adapters", "app", "testing"].iter().any(|c| *l == *c));
            let in_tests = clustered
                && parts.get(2).is_some_and(|d| *d == "tests")
                && p.extension().is_some_and(|e| e == "rs");
            let fixture = relative.starts_with("testing/oxikube_testkit/fixtures/cluster")
                && p.extension().is_some_and(|e| e == "yaml" || e == "yml");
            in_tests || fixture
        })
        .collect()
}

/// Whether `literal` is an image reference rather than `host:port`: `name:tag` where the name has
/// a path (`registry/repo`) or the tag is not just digits.
fn looks_like_an_image(literal: &str) -> bool {
    is_image_reference(literal)
        && literal.rsplit_once(':').is_some_and(|(name, tag)| {
            name.contains('/') || !tag.chars().all(|c| c.is_ascii_digit())
        })
}

/// The image references written on `line`: any image-like string literal in a Rust line, or the
/// value of a YAML `image:` key.
fn image_literals(line: &str, yaml: bool) -> Vec<String> {
    if yaml {
        return line
            .trim()
            .strip_prefix("image:")
            .map(|v| vec![v.trim().trim_matches('"').to_owned()])
            .unwrap_or_default();
    }
    if line.trim_start().starts_with("//") {
        return Vec::new();
    }
    line.split('"')
        .skip(1)
        .step_by(2)
        .filter(|literal| looks_like_an_image(literal))
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_image_an_integration_test_or_fixture_runs_is_in_the_list() {
    let listed = images::all();
    let mut missing = Vec::new();
    let mut seen = 0;
    for path in integration_sources() {
        // This file's own sample literals are data for the scanner, not images a test runs.
        if path.ends_with("oxikube_testkit/tests/test_images.rs") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let yaml = path.extension().is_some_and(|e| e == "yaml" || e == "yml");
        for (n, line) in text.lines().enumerate() {
            for image in image_literals(line, yaml) {
                if is_deliberately_unpullable(&image) {
                    continue;
                }
                seen += 1;
                if !listed.contains(&image.as_str()) {
                    missing.push(format!("{}:{}: {image}", path.display(), n + 1));
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "images used by integration tests or fixtures but missing from \
         crates/testing/oxikube_testkit/fixtures/test-images.txt (kind-up would not pull them):\n{}",
        missing.join("\n")
    );
    // The scan finds the cluster fixtures' images at least; an empty scan means it is broken.
    assert!(seen >= 4, "the scan found only {seen} image literals");
}

#[test]
fn image_literals_are_found_in_rust_and_yaml() {
    assert_eq!(
        image_literals(r#"{"name": "a", "image": "busybox:1.37"}"#, false),
        ["busybox:1.37"]
    );
    assert_eq!(
        image_literals("      image: registry.k8s.io/pause:3.10", true),
        ["registry.k8s.io/pause:3.10"]
    );
    // A constant is found without the word "image" on its line.
    assert_eq!(
        image_literals(r#"pub const TOOLS: &str = "alpine:3.20";"#, false),
        ["alpine:3.20"]
    );
    // Not an image: a host and port, a comment, or not a reference.
    assert!(image_literals(r#"let host = "127.0.0.1:0";"#, false).is_empty());
    assert!(image_literals(r#"// the image "busybox:1.37""#, false).is_empty());
    assert!(image_literals(r#"assert_eq!(image, "no tag here");"#, false).is_empty());
}
