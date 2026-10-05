//! `cargo xtask gen-settings-schema [--check]`: write or verify `settings.schema.json`.
//!
//! The schema comes from the registered settings types, so it is produced by Rust code that
//! links them: the `oxikube_settings` example `settings_schema` prints it (xtask does not
//! depend on the GPUI-based settings crate, which keeps the pre-commit hook fast). The output
//! is canonical (keys sorted), so `--check` is a byte comparison; CI runs it so a settings
//! change without a regenerated schema fails.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Where the schema lives, relative to the workspace root (mirrors
/// `oxikube_assets::SETTINGS_SCHEMA_PATH`).
pub const SCHEMA_PATH: &str = "crates/platform/oxikube_assets/assets/settings/settings.schema.json";

/// Package and example that print the schema.
const GENERATOR: (&str, &str) = ("oxikube_settings", "settings_schema");

#[derive(clap::Args)]
pub struct Args {
    /// Fail if the checked-in schema differs from a fresh generation instead of writing it.
    #[arg(long)]
    pub check: bool,
}

pub fn run(args: &Args) -> Result<()> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .no_deps()
        .exec()
        .context("cargo metadata")?;
    let root = metadata.workspace_root.as_std_path();
    let generated = generate(root)?;
    let path = root.join(SCHEMA_PATH);
    if args.check {
        let existing = std::fs::read_to_string(&path).ok();
        check(existing.as_deref(), &generated)?;
        println!("{SCHEMA_PATH} is up to date");
    } else {
        std::fs::write(&path, &generated).with_context(|| format!("writing {}", path.display()))?;
        println!("wrote {SCHEMA_PATH}");
    }
    Ok(())
}

/// Run the generator example and return its stdout.
fn generate(root: &Path) -> Result<String> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let (package, example) = GENERATOR;
    let output = Command::new(cargo)
        .current_dir(root)
        .args(["run", "-q", "-p", package, "--example", example])
        .output()
        .context("running the settings schema generator")?;
    if !output.status.success() {
        bail!(
            "settings schema generator failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let text = String::from_utf8(output.stdout).context("schema is not UTF-8")?;
    serde_json::from_str::<serde_json::Value>(&text).context("generator printed invalid JSON")?;
    Ok(text)
}

/// The `--check` verdict: the checked-in text must equal the generated text.
pub fn check(existing: Option<&str>, generated: &str) -> Result<()> {
    match existing {
        None => bail!("{SCHEMA_PATH} is missing; run `cargo xtask gen-settings-schema`"),
        Some(existing) if existing != generated => bail!(
            "{SCHEMA_PATH} is stale (a settings type changed); run `cargo xtask gen-settings-schema` and commit the result"
        ),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_passes_only_on_an_identical_schema() {
        let schema = "{\n  \"type\": \"object\"\n}\n";
        assert!(check(Some(schema), schema).is_ok());

        let stale = check(Some("{}\n"), schema).unwrap_err().to_string();
        assert!(stale.contains("stale"), "{stale}");

        let missing = check(None, schema).unwrap_err().to_string();
        assert!(missing.contains("missing"), "{missing}");
    }

    #[test]
    fn schema_path_matches_the_assets_crate() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let assets =
            std::fs::read_to_string(root.join("crates/platform/oxikube_assets/src/settings.rs"))
                .unwrap();
        assert!(assets.contains(&format!("\"{SCHEMA_PATH}\"")));
        assert!(root.join(SCHEMA_PATH).exists());
    }
}
