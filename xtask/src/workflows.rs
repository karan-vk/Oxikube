//! Guards on `.github/workflows` (E01-F542): CI builds with the toolchain pinned in
//! `rust-toolchain.toml`, and the PR workflow keeps the gates the nightly also runs, so a break
//! shows on the PR instead of days later. Test-only: there is no subcommand.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the repo root")
        .to_owned()
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

/// Every workflow file's name and text.
fn workflows() -> Vec<(String, String)> {
    let dir = repo_root().join(".github/workflows");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("workflows dir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "yml"))
        .map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let text = read(&format!(".github/workflows/{name}"));
            (name, text)
        })
        .collect();
    files.sort();
    files
}

#[test]
fn no_workflow_installs_a_floating_toolchain() {
    for (name, text) in workflows() {
        assert!(
            !text.contains("dtolnay/rust-toolchain"),
            "{name} installs a floating toolchain; use ./.github/actions/setup-rust, which honours \
             rust-toolchain.toml (#402)"
        );
    }
}

#[test]
fn the_setup_action_installs_what_rust_toolchain_toml_pins() {
    let action = read(".github/actions/setup-rust/action.yml");
    assert!(
        action.contains("rustup toolchain install"),
        "setup-rust must install the pinned toolchain"
    );
    assert!(
        !action.contains("rustup toolchain install stable") && !action.contains("--default"),
        "setup-rust must not name a channel: rust-toolchain.toml is the only pin"
    );
    assert!(
        read("rust-toolchain.toml").contains("channel = \""),
        "rust-toolchain.toml pins a channel"
    );
    // Every workflow that builds Rust uses it.
    for (name, text) in workflows() {
        if text.contains("Swatinem/rust-cache") {
            assert!(
                text.contains("./.github/actions/setup-rust"),
                "{name} builds Rust without setup-rust"
            );
        }
    }
}

#[test]
fn pr_ci_runs_the_gates_the_nightly_used_to_find_alone() {
    let ci = read(".github/workflows/ci.yml");
    for (needle, why) in [
        (
            "--features perf-scenarios --test perf_scenario",
            "the perf-scenarios smoke test",
        ),
        (
            "oxikube_terminal/screenshot",
            "screenshot-feature clippy for oxikube_terminal",
        ),
        (
            "oxikube_logs_ui/screenshot",
            "screenshot-feature clippy for oxikube_logs_ui",
        ),
        (
            "oxikube/perf-scenarios",
            "perf-scenarios feature clippy for oxikube",
        ),
        ("cargo doc --workspace --no-deps", "rustdoc"),
        (
            "RUSTDOCFLAGS: \"-D warnings\"",
            "rustdoc warnings as errors",
        ),
    ] {
        assert!(ci.contains(needle), "ci.yml lost {why} (`{needle}`)");
    }
}
