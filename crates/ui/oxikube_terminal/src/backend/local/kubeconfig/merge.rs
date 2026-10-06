//! Reading kubeconfig files and cutting the selected context out of them.

use std::path::{Path, PathBuf};

use oxikube_domain::{OxiError, OxiResult};
use serde_json::{Map, Value, json};

use super::ClusterEnv;

/// The merged document and the namespace the shell starts in.
pub struct Merged {
    /// A kubeconfig (JSON, which every kubeconfig reader accepts as YAML) with one context.
    pub text: String,
    /// The namespace stored in the context (`default` when nothing names one).
    pub namespace: String,
}

/// One kubeconfig file, parsed. `dir` resolves its relative paths.
struct Parsed {
    dir: PathBuf,
    doc: Value,
}

/// Cuts `env.context` out of `env.kubeconfig_files`: the context, the cluster and the user it
/// names (each from the first file that defines that name), with `current-context` set,
/// the namespace applied, and every path the entries reference made absolute (kubectl resolves
/// them against the defining file, the merged file lives elsewhere).
///
/// # Errors
///
/// See [`ClusterEnv::prepare`].
pub fn merged_kubeconfig(env: &ClusterEnv) -> OxiResult<Merged> {
    let files = parse_files(&env.kubeconfig_files);
    let (_, context) = find(&files, "contexts", env.context.as_str())
        .ok_or_else(|| OxiError::not_found("no kubeconfig file defines this context"))?;
    let details = context.get("context").cloned().unwrap_or(Value::Null);
    let namespace = env
        .namespace
        .clone()
        .or_else(|| text(&details, "namespace"))
        .unwrap_or_else(|| "default".to_owned());

    let mut merged_context = Map::new();
    let mut clusters = Vec::new();
    let mut users = Vec::new();
    for (key, section, bucket) in [
        ("cluster", "clusters", &mut clusters),
        ("user", "users", &mut users),
    ] {
        let Some(name) = text(&details, key) else {
            continue;
        };
        merged_context.insert(key.to_owned(), Value::String(name.clone()));
        // The defining file may be another one than the context's.
        if let Some((file, entry)) = find(&files, section, &name) {
            bucket.push(absolutised(entry, key, &file.dir));
        }
    }
    merged_context.insert("namespace".into(), Value::String(namespace.clone()));

    let doc = json!({
        "apiVersion": "v1",
        "kind": "Config",
        "current-context": env.context.as_str(),
        "preferences": {},
        "contexts": [{ "name": env.context.as_str(), "context": merged_context }],
        "clusters": clusters,
        "users": users,
    });
    let text = serde_json::to_string_pretty(&doc)
        .map_err(|_| OxiError::internal("could not write the merged kubeconfig"))?;
    Ok(Merged { text, namespace })
}

/// Parses each readable file; one that cannot be read or parsed is skipped (the catalog
/// already reports it), so a broken file does not stop a terminal for another cluster.
fn parse_files(paths: &[PathBuf]) -> Vec<Parsed> {
    paths
        .iter()
        .filter_map(|path| {
            let raw = std::fs::read_to_string(path).ok()?;
            let doc: Value = serde_saphyr::from_str(&raw).ok()?;
            let dir = path.parent().map(Path::to_owned).unwrap_or_default();
            Some(Parsed { dir, doc })
        })
        .collect()
}

/// The first entry named `name` in the `section` list of the files, with the file it is from.
fn find<'a>(files: &'a [Parsed], section: &str, name: &str) -> Option<(&'a Parsed, &'a Value)> {
    files.iter().find_map(|file| {
        let entry = file
            .doc
            .get(section)?
            .as_array()?
            .iter()
            .find(|entry| entry.get("name").and_then(Value::as_str) == Some(name))?;
        Some((file, entry))
    })
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// `entry` (`{name, cluster|user: {...}}`) with its file references resolved against `dir`.
fn absolutised(entry: &Value, key: &str, dir: &Path) -> Value {
    let mut entry = entry.clone();
    if let Some(body) = entry.get_mut(key) {
        for field in PATH_FIELDS {
            absolutise_field(body, field, dir);
        }
        // Exec plugins and auth-provider helpers: a relative path runs from the file's folder;
        // a bare name is looked up on PATH and stays as it is.
        for (parent, field) in [("exec", "command"), ("auth-provider.config", "cmd-path")] {
            let mut node = Some(&mut *body);
            for part in parent.split('.') {
                node = node.and_then(|n| n.get_mut(part));
            }
            if let Some(node) = node {
                if node
                    .get(field)
                    .and_then(Value::as_str)
                    .is_some_and(|c| c.contains('/'))
                {
                    absolutise_field(node, field, dir);
                }
            }
        }
    }
    entry
}

/// Fields that hold file paths, directly under a cluster or user body.
const PATH_FIELDS: [&str; 4] = [
    "certificate-authority",
    "client-certificate",
    "client-key",
    "tokenFile",
];

fn absolutise_field(node: &mut Value, field: &str, dir: &Path) {
    let Some(slot) = node.get_mut(field) else {
        return;
    };
    if let Some(path) = slot.as_str().map(Path::new).filter(|p| p.is_relative()) {
        *slot = Value::String(dir.join(path).to_string_lossy().into_owned());
    }
}
