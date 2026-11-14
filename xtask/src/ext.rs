//! `cargo xtask ext new <id> [--kind command|mcp|theme] [--dir <path>]`: scaffold an extension
//! (E23-S10).
//!
//! The templates are plain text in `xtask/templates/ext/` with `{{name}}` placeholders, filled by
//! [`render`]; there is no template engine.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

/// `oxikube_extension_api` version written to `[lib] version` in the manifest.
const API_VERSION: &str = "0.1.0";
/// Where the SDK crate lives, relative to the repository root.
const API_CRATE: &str = "crates/platform/oxikube_extension_api";

#[derive(clap::Subcommand)]
pub enum Cmd {
    /// Scaffold a new extension project (manifest, lib.rs, build script).
    New(NewArgs),
}

#[derive(clap::Args)]
pub struct NewArgs {
    /// Extension id, `[a-z0-9-]+`.
    id: String,
    /// What the extension contributes.
    #[arg(long, value_enum, default_value = "command")]
    kind: Kind,
    /// Directory to create; defaults to `./<id>`.
    #[arg(long)]
    dir: Option<PathBuf>,
}

#[derive(clap::ValueEnum, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A palette command (Rust component).
    Command,
    /// An MCP context server (Rust component).
    Mcp,
    /// A theme only, no code.
    Theme,
}

pub fn run(cmd: &Cmd) -> Result<()> {
    let Cmd::New(args) = cmd;
    let dir = args.dir.clone().unwrap_or_else(|| PathBuf::from(&args.id));
    let dir = std::path::absolute(&dir).context("resolving the target directory")?;
    // This file is `<repo>/xtask/src/ext.rs`, so the repo root is two levels above the manifest dir.
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("repo root")?;
    scaffold(&args.id, args.kind, &dir, repo)?;
    println!("created {}", dir.display());
    Ok(())
}

/// Checks the id against the manifest rule (`[a-z0-9-]+`, not starting or ending with `-`).
fn validate_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-');
    if !ok {
        bail!(
            "invalid extension id `{id}`: use lowercase letters, digits and `-` (not at the ends)"
        );
    }
    Ok(())
}

/// Replaces every `{{key}}` of `template` with its value in `vars`.
fn render(template: &str, vars: &[(&str, &str)]) -> String {
    vars.iter()
        .fold(template.to_string(), |text, (key, value)| {
            text.replace(&format!("{{{{{key}}}}}"), value)
        })
}

/// The SDK dependency: a relative path when `dir` is inside the repo, else the absolute path of
/// this checkout (the SDK is not published to a registry).
fn api_dependency(dir: &Path, repo: &Path) -> String {
    let api = repo.join(API_CRATE);
    let path = match dir.strip_prefix(repo) {
        Ok(inside) => {
            let up = "../".repeat(inside.components().count());
            format!("{up}{API_CRATE}")
        }
        Err(_) => api.display().to_string(),
    };
    format!("{{ path = {path:?} }}")
}

/// `hello-world` -> `HelloWorld`, the extension's Rust type name.
fn type_name(id: &str) -> String {
    let camel: String = id
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| format!("{}{}", part[..1].to_uppercase(), &part[1..]))
        .collect();
    if camel.starts_with(|c: char| c.is_ascii_digit()) {
        format!("Ext{camel}")
    } else {
        camel
    }
}

/// Writes the project for `id` into `dir`, which must not exist yet.
fn scaffold(id: &str, kind: Kind, dir: &Path, repo: &Path) -> Result<()> {
    validate_id(id)?;
    if dir.exists() {
        bail!("{} already exists", dir.display());
    }
    let api_dependency = api_dependency(dir, repo);
    let type_name = type_name(id);
    let (body, build_section) = match kind {
        Kind::Command => (include_str!("../templates/ext/body_command.toml"), true),
        Kind::Mcp => (include_str!("../templates/ext/body_mcp.toml"), true),
        Kind::Theme => (include_str!("../templates/ext/body_theme.toml"), false),
    };
    let section = if build_section {
        include_str!("../templates/ext/build_section_lib.md")
    } else {
        include_str!("../templates/ext/build_section_theme.md")
    };
    let vars = [
        ("id", id),
        ("type_name", type_name.as_str()),
        ("api_version", API_VERSION),
        ("api_dependency", api_dependency.as_str()),
    ];
    let body = render(body, &vars);
    let section = render(section, &vars);
    let vars = [
        vars.as_slice(),
        &[("body", body.as_str()), ("build_section", section.as_str())],
    ]
    .concat();

    let mut files: Vec<(String, String)> = vec![
        (
            "extension.toml".into(),
            include_str!("../templates/ext/extension.toml").into(),
        ),
        (
            "README.md".into(),
            include_str!("../templates/ext/README.md").into(),
        ),
        (
            ".gitignore".into(),
            include_str!("../templates/ext/gitignore").into(),
        ),
    ];
    match kind {
        Kind::Theme => {
            files.push((
                format!("themes/{id}.json"),
                include_str!("../templates/ext/theme.json").into(),
            ));
        }
        Kind::Command | Kind::Mcp => {
            let lib = match kind {
                Kind::Mcp => include_str!("../templates/ext/lib_mcp.rs"),
                _ => include_str!("../templates/ext/lib_command.rs"),
            };
            files.extend([
                (
                    "Cargo.toml".into(),
                    include_str!("../templates/ext/Cargo.toml").into(),
                ),
                ("src/lib.rs".into(), lib.into()),
                (
                    "build.sh".into(),
                    include_str!("../templates/ext/build.sh").into(),
                ),
                (
                    ".cargo/config.toml".into(),
                    include_str!("../templates/ext/cargo_config.toml").into(),
                ),
            ]);
        }
    }
    for (name, template) in files {
        let path = dir.join(&name);
        std::fs::create_dir_all(path.parent().context("file has a parent")?)?;
        std::fs::write(&path, render(&template, &vars))
            .with_context(|| format!("writing {}", path.display()))?;
        #[cfg(unix)]
        if name == "build.sh" {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
