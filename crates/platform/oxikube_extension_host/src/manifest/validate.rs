//! Validation rules that serde cannot express.

use std::collections::HashSet;

use super::{ExtensionManifest, ManifestError, SCHEMA_VERSION};

pub(super) fn validate(m: &ExtensionManifest) -> Result<(), ManifestError> {
    check_id("id", &m.id)?;
    if m.name.trim().is_empty() {
        return Err(ManifestError::invalid("name", "must not be empty"));
    }
    if m.schema_version != SCHEMA_VERSION {
        return Err(ManifestError::invalid(
            "schema_version",
            format!(
                "{} is not supported, expected {SCHEMA_VERSION}",
                m.schema_version
            ),
        ));
    }
    for path in &m.themes {
        check_relative_path("themes", path)?;
    }
    for path in &m.icon_themes {
        check_relative_path("icon_themes", path)?;
    }
    let mut seen = HashSet::new();
    for command in &m.commands {
        check_id("commands.id", &command.id)?;
        if command.title.trim().is_empty() {
            return Err(ManifestError::invalid(
                "commands.title",
                "must not be empty",
            ));
        }
        if !seen.insert(command.id.as_str()) {
            return Err(ManifestError::invalid(
                "commands.id",
                format!("duplicate id `{}`", command.id),
            ));
        }
    }
    seen.clear();
    for server in &m.context_servers {
        check_id("context_servers.id", &server.id)?;
        if !seen.insert(server.id.as_str()) {
            return Err(ManifestError::invalid(
                "context_servers.id",
                format!("duplicate id `{}`", server.id),
            ));
        }
    }
    for capability in &m.capabilities {
        let patterns = capability.patterns();
        if patterns.is_empty() || patterns.iter().any(|p| p.trim().is_empty()) {
            return Err(ManifestError::invalid(
                format!("capabilities.{}", capability.kind()),
                "patterns must be non-empty",
            ));
        }
    }
    Ok(())
}

/// `[a-z0-9-]+`; this also rejects `:`, which the host uses to namespace commands.
fn check_id(field: &str, id: &str) -> Result<(), ManifestError> {
    let ok = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if ok {
        Ok(())
    } else {
        Err(ManifestError::invalid(
            field,
            format!("`{id}` must match [a-z0-9-]+"),
        ))
    }
}

/// Relative, no `..`, so the file stays inside the extension directory.
fn check_relative_path(field: &str, path: &str) -> Result<(), ManifestError> {
    let p = std::path::Path::new(path);
    let escapes = p.is_absolute()
        || path.starts_with('/')
        || p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        });
    if path.is_empty() || escapes {
        Err(ManifestError::invalid(
            field,
            format!("`{path}` must be a relative path inside the extension"),
        ))
    } else {
        Ok(())
    }
}
