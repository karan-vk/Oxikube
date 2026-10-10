//! `scripts/gpui-overlay.sh` against a fixture repository: a fake pinned crate in a fake cargo
//! download cache, its checksum, a lockfile and patch files. Needs bash, tar and git (as the script
//! does; CI has them on macOS and Linux).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

const CRATE: &str = "gpui-pre-fake";
const VERSION: &str = "0.1.0";
const NAME: &str = "gpui-pre-fake-0.1.0";

const FIX: &str = "Fix the answer.\nUpstream: none\nPatch: GPL-3.0-or-later\n\n\
diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n\
-pub const ANSWER: u32 = 41;\n+pub const ANSWER: u32 = 42;\n";

const STALE: &str = "Does not apply.\nUpstream: none\nPatch: GPL-3.0-or-later\n\n\
diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n\
-pub const ANSWER: u32 = 7;\n+pub const ANSWER: u32 = 8;\n";

struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    cargo_home: PathBuf,
}

impl Fixture {
    /// A repository with `patches/gpui/gpui-pre-fake-0.1.0/0001-fix.patch` and the pinned crate
    /// in the download cache.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let cargo_home = dir.path().join("cargo");
        let src = dir.path().join("src").join(NAME);
        fs::create_dir_all(src.join("src")).unwrap();
        fs::write(
            src.join("Cargo.toml"),
            format!("[package]\nname = \"{CRATE}\"\nversion = \"{VERSION}\"\n"),
        )
        .unwrap();
        fs::write(src.join("src/lib.rs"), "pub const ANSWER: u32 = 41;\n").unwrap();
        let cache = cargo_home.join("registry/cache/index.crates.io-test");
        fs::create_dir_all(&cache).unwrap();
        let archive = cache.join(format!("{NAME}.crate"));
        run_ok(
            Command::new("tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(dir.path().join("src"))
                .arg(NAME),
        );
        let sum = sha256(&archive);

        let patches = root.join("patches/gpui").join(NAME);
        fs::create_dir_all(&patches).unwrap();
        fs::write(patches.join("checksum"), format!("{sum}\n")).unwrap();
        fs::write(patches.join("0001-fix.patch"), FIX).unwrap();
        fs::write(
            root.join("Cargo.lock"),
            format!("version = 4\n\n[[package]]\nname = \"{CRATE}\"\nversion = \"{VERSION}\"\n"),
        )
        .unwrap();
        Fixture {
            _dir: dir,
            root,
            cargo_home,
        }
    }

    fn script(&self, args: &[&str]) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/gpui-overlay.sh");
        Command::new("bash")
            .arg(script)
            .args(args)
            .env("GPUI_OVERLAY_ROOT", &self.root)
            .env("CARGO_HOME", &self.cargo_home)
            .output()
            .unwrap()
    }

    fn overlay(&self) -> PathBuf {
        self.root.join(".gpui-overlay").join(NAME)
    }

    fn patches(&self) -> PathBuf {
        self.root.join("patches/gpui").join(NAME)
    }
}

fn run_ok(command: &mut Command) {
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn sha256(path: &Path) -> String {
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .or_else(|_| Command::new("sha256sum").arg(path).output())
        .unwrap();
    String::from_utf8(out.stdout).unwrap()[..64].to_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn builds_the_pinned_crate_with_its_patches_once() {
    let fx = Fixture::new();
    let first = fx.script(&[]);
    assert!(first.status.success(), "{}", stderr(&first));
    let lib = fs::read_to_string(fx.overlay().join("src/lib.rs")).unwrap();
    assert_eq!(lib, "pub const ANSWER: u32 = 42;\n", "the patch is applied");
    assert!(fx.overlay().join("Cargo.toml").exists());

    let stamped = fs::metadata(fx.overlay().join("src/lib.rs"))
        .unwrap()
        .modified()
        .unwrap();
    let again = fx.script(&[]);
    assert!(again.status.success(), "{}", stderr(&again));
    assert!(again.stdout.is_empty(), "a second run is a no-op");
    let after = fs::metadata(fx.overlay().join("src/lib.rs"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(stamped, after, "nothing is rewritten when nothing changed");

    let check = fx.script(&["--check"]);
    assert!(check.status.success(), "{}", stderr(&check));
}

#[test]
fn check_finds_a_missing_stale_or_hand_edited_overlay() {
    let fx = Fixture::new();
    let missing = fx.script(&["--check"]);
    assert!(!missing.status.success());
    assert!(stderr(&missing).contains("is missing; run scripts/gpui-overlay.sh"));

    assert!(fx.script(&[]).status.success());
    fs::write(
        fx.overlay().join("src/lib.rs"),
        "pub const ANSWER: u32 = 0;\n",
    )
    .unwrap();
    let edited = fx.script(&["--check"]);
    assert!(!edited.status.success());
    assert!(stderr(&edited).contains("differs from the pinned crate plus its patches"));

    fs::write(fx.patches().join("0002-more.patch"), FIX).unwrap();
    let stale = fx.script(&["--check"]);
    assert!(!stale.status.success());
    assert!(stderr(&stale).contains("is stale"));

    fs::create_dir_all(fx.root.join(".gpui-overlay/gpui-pre-other-0.1.0")).unwrap();
    fs::remove_file(fx.patches().join("0002-more.patch")).unwrap();
    let extra = fx.script(&["--check"]);
    assert!(
        stderr(&extra).contains("gpui-pre-other-0.1.0 has no patches/gpui/gpui-pre-other-0.1.0")
    );
    // A build run rebuilds the edited crate and removes what has no patch directory.
    assert!(fx.script(&[]).status.success());
    assert!(!fx.root.join(".gpui-overlay/gpui-pre-other-0.1.0").exists());
}

#[test]
fn a_patch_that_does_not_apply_fails_loudly_and_leaves_no_overlay() {
    let fx = Fixture::new();
    assert!(fx.script(&[]).status.success());
    fs::write(fx.patches().join("0002-stale.patch"), STALE).unwrap();
    let out = fx.script(&[]);
    assert!(!out.status.success());
    let err = stderr(&out);
    assert!(
        err.contains("0002-stale.patch does not apply to the pinned gpui-pre-fake 0.1.0"),
        "{err}"
    );
    assert!(
        !fx.overlay().exists(),
        "the previous overlay is gone, so cargo fails instead of building it"
    );
}

#[test]
fn the_crate_must_match_its_checksum_and_the_lockfile_version() {
    let fx = Fixture::new();
    let other = "0".repeat(64);
    fs::write(fx.patches().join("checksum"), &other).unwrap();
    // The cache entry does not match, so the script downloads from crates.io; point it at a host
    // that fails fast instead.
    let out = Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/gpui-overlay.sh"))
        .env("GPUI_OVERLAY_ROOT", &fx.root)
        .env("CARGO_HOME", &fx.cargo_home)
        .env("https_proxy", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("could not download"),
        "{}",
        stderr(&out)
    );

    let fx = Fixture::new();
    fs::write(
        fx.root.join("Cargo.lock"),
        format!("version = 4\n\n[[package]]\nname = \"{CRATE}\"\nversion = \"0.2.0\"\n"),
    )
    .unwrap();
    let out = fx.script(&[]);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains("Cargo.lock pins gpui-pre-fake at 0.2.0"),
        "{}",
        stderr(&out)
    );
}
