//! Install git pre-commit and pre-push hooks via `pre-commit`.

use std::path::Path;

use anyhow::{Context, Result, bail};
use xshell::{Shell, cmd};

pub fn run() -> Result<()> {
    let sh = Shell::new()?;

    // Check if `pre-commit` is on PATH.
    if cmd!(sh, "pre-commit --version").read().is_err() {
        eprintln!(
            "error: `pre-commit` was not found on PATH.\n\
             Install it using one of the following:\n  \
             brew install pre-commit\n  \
             pipx install pre-commit\n  \
             uv tool install pre-commit"
        );
        bail!("pre-commit is not installed or not in PATH");
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest_dir
        .parent()
        .context("failed to determine repository root from CARGO_MANIFEST_DIR")?;

    sh.change_dir(root);

    cmd!(
        sh,
        "pre-commit install --install-hooks --hook-type pre-commit --hook-type pre-push"
    )
    .run()
    .context("failed to run `pre-commit install`")?;

    println!("pre-commit and pre-push hooks successfully installed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_root_contains_cargo_manifest() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let root = manifest_dir.parent().expect("repo root parent");
        assert!(root.join("Cargo.toml").exists());
        assert!(root.join(".pre-commit-config.yaml").exists());
    }
}
