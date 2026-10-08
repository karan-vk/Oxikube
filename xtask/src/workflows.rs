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

/// The screenshot goldens the nightly Linux leg compares (E05-F453): without a committed Linux
/// golden a test runs its structural checks only and cannot see a pixel regression. Each is
/// `<crate>/tests/goldens/linux/<name>.png`, beside the macOS golden of the same test.
const LINUX_GOLDENS: &[&str] = &[
    "crates/ui/oxikube_ui/tests/goldens/linux/token_sampler.png",
    "bins/oxikube/tests/goldens/linux/main_window.png",
    "crates/ui/oxikube_workspace/tests/goldens/linux/workspace.png",
    "crates/ui/oxikube_workspace/tests/goldens/linux/workspace_modal.png",
    "crates/ui/oxikube_workspace/tests/goldens/linux/main_window_restoring.png",
];

/// Pixel size of a PNG, read from its IHDR chunk (always the first chunk).
fn png_size(relative: &str) -> (u32, u32) {
    let path = repo_root().join(relative);
    let bytes = fs::read(&path).unwrap_or_else(|err| {
        panic!(
            "{relative} is missing ({err}); refresh it with the refresh-goldens workflow \
             (docs/testing-gpui.md)"
        )
    });
    assert!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.get(12..16) == Some(b"IHDR"),
        "{relative} is not a PNG"
    );
    let be = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().expect("4 bytes"));
    (be(16), be(20))
}

#[test]
fn linux_goldens_are_committed_for_the_nightly_screenshot_tests() {
    for linux in LINUX_GOLDENS {
        let macos = linux.replace("/goldens/linux/", "/goldens/macos/");
        // Same test, same window: a golden of another size would fail every nightly comparison.
        assert_eq!(
            png_size(linux),
            png_size(&macos),
            "{linux} is not the size of {macos}"
        );
    }
}

#[test]
fn refresh_goldens_regenerates_and_verifies_what_the_nightly_compares() {
    let refresh = read(".github/workflows/refresh-goldens.yml");
    let nightly = read(".github/workflows/nightly.yml");
    for command in [
        "-p oxikube --features screenshot --test screenshot",
        "-p oxikube_ui --features screenshot --test screenshot",
        "-p oxikube_workspace --features screenshot --test screenshot",
    ] {
        // Once in update mode, once in compare mode.
        assert_eq!(
            refresh.matches(command).count(),
            2,
            "refresh-goldens must regenerate and then verify with `{command}`"
        );
        assert!(
            nightly.contains(command)
                || nightly.contains(&command.replace(" --test screenshot", "")),
            "nightly no longer runs `{command}`"
        );
    }
    assert!(
        refresh.contains("OXIKUBE_UPDATE_GOLDENS"),
        "refresh-goldens must run the tests in update mode"
    );
    assert!(
        !refresh.contains("push:"),
        "refresh-goldens is dispatch-only; drop the bootstrap push trigger"
    );
}

/// The nightly's `run:` lines that invoke `cargo test`, whitespace-normalised.
fn nightly_cargo_test_lines() -> Vec<String> {
    read(".github/workflows/nightly.yml")
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| line.contains("cargo test "))
        .collect()
}

/// Whether `lines` has a `cargo test` for `package` with `--features <feature>` that runs `target`
/// (a named `--test`, or every test of the package when the line names none).
fn nightly_runs(lines: &[String], package: &str, feature: &str, target: Option<&str>) -> bool {
    lines.iter().any(|line| {
        let words: Vec<&str> = line.split(' ').collect();
        let follows = |flag: &str, value: &str| {
            words
                .windows(2)
                .any(|pair| pair[0] == flag && pair[1] == value)
        };
        let named_tests: Vec<&str> = words
            .windows(2)
            .filter(|pair| pair[0] == "--test")
            .map(|pair| pair[1])
            .collect();
        follows("-p", package)
            && follows("--features", feature)
            && match target {
                Some(name) => named_tests.is_empty() || named_tests.contains(&name),
                None => true,
            }
    })
}

/// A feature name that gates a headless GPU render: `screenshot`, `gpui-golden`, ...
fn is_render_feature(feature: &str) -> bool {
    feature.contains("screenshot") || feature.contains("golden")
}

/// Every test gated behind a render feature (`required-features` in a `[[test]]`) must be run by
/// the nightly's screenshot step (E01-F583): the list is hand-written, and a new screenshot test
/// that is not added to it never runs anywhere, so its goldens rot unseen.
#[test]
fn nightly_runs_every_screenshot_test() {
    let metadata = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .current_dir(repo_root())
        .exec()
        .expect("cargo metadata");
    let lines = nightly_cargo_test_lines();
    let mut missing = Vec::new();
    for package in metadata.workspace_packages() {
        // A crate whose tests are gated with `#[cfg(feature = "screenshot")]` rather than
        // `required-features` still has to appear in the step. (Only the `screenshot` feature
        // itself: `gpui-screenshot` and friends are helper features that dev-dependencies turn on.)
        for feature in package.features.keys().filter(|f| *f == "screenshot") {
            if !nightly_runs(&lines, &package.name, feature, None) {
                missing.push(format!("-p {} --features {feature}", package.name));
            }
        }
        for target in package
            .targets
            .iter()
            .filter(|t| t.is_kind(cargo_metadata::TargetKind::Test))
        {
            for feature in target
                .required_features
                .iter()
                .filter(|f| is_render_feature(f))
            {
                if !nightly_runs(&lines, &package.name, feature, Some(&target.name)) {
                    missing.push(format!(
                        "-p {} --features {feature} --test {}",
                        package.name, target.name
                    ));
                }
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "nightly.yml's screenshot step does not run: {missing:#?}\nadd a `cargo test` line for each \
         (check it is green on the runners first)"
    );
}

#[test]
fn the_screenshot_step_matcher_reads_package_feature_and_test() {
    let lines = vec![
        "${{ runner.os == 'Linux' && 'xvfb-run -a' || '' }} cargo test -p a --features screenshot --test one --profile x"
            .to_owned(),
        "cargo test -p b --features screenshot --profile x".to_owned(),
    ];
    assert!(nightly_runs(&lines, "a", "screenshot", Some("one")));
    assert!(!nightly_runs(&lines, "a", "screenshot", Some("two")));
    assert!(!nightly_runs(&lines, "a", "golden", Some("one")));
    assert!(!nightly_runs(&lines, "ab", "screenshot", Some("one")));
    assert!(nightly_runs(&lines, "b", "screenshot", Some("anything")));
    assert!(nightly_runs(&lines, "b", "screenshot", None));
    assert!(!nightly_runs(&lines, "c", "screenshot", None));
}
