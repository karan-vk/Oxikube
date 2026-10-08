//! [`WorldDescribe`]: the Describe tab's text for a synthetic cluster's objects, in `kubectl
//! describe`'s shape (`Key:  value`, nested maps indented, a ConfigMap's data under `Data`), so
//! describing the 5 MB ConfigMap hands the tab about 5 MB of text, as kubectl would.

use std::fmt::Write as _;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ResourceRef;
use oxikube_ports::{DescribeOutput, DescribePort, DescribeSource, ResourcePort};
use serde_json::Value;

/// See the [module docs](self).
pub struct WorldDescribe {
    resources: Arc<dyn ResourcePort>,
}

impl WorldDescribe {
    /// Describes the objects `resources` reads.
    pub fn new(resources: Arc<dyn ResourcePort>) -> Self {
        Self { resources }
    }
}

#[async_trait]
impl DescribePort for WorldDescribe {
    async fn describe(&self, target: &ResourceRef) -> OxiResult<DescribeOutput> {
        let object = self
            .resources
            .get(&target.gvk, target.namespace(), &target.name)
            .await?;
        Ok(DescribeOutput {
            text: describe(&object.json),
            source: DescribeSource::Native,
        })
    }
}

/// `object` as `kubectl describe` lays one out.
pub fn describe(object: &Value) -> String {
    let mut out = String::new();
    let meta = &object["metadata"];
    let field = |out: &mut String, key: &str, value: &Value| {
        let text = value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_owned);
        let _ = writeln!(out, "{key:<14}{text}");
    };
    field(&mut out, "Name:", &meta["name"]);
    if !meta["namespace"].is_null() {
        field(&mut out, "Namespace:", &meta["namespace"]);
    }
    tree(&mut out, "Labels:", &meta["labels"], 0);
    tree(&mut out, "Annotations:", &meta["annotations"], 0);
    if let Some(data) = object["data"].as_object() {
        out.push_str("\nData\n====\n");
        for (key, value) in data {
            let _ = writeln!(
                out,
                "{key}:\n----\n{}\n",
                value.as_str().unwrap_or_default()
            );
        }
    }
    for section in ["spec", "status"] {
        if !object[section].is_null() {
            let title = format!("{}{}:", section[..1].to_uppercase(), &section[1..]);
            tree(&mut out, &title, &object[section], 0);
        }
    }
    out.push_str("Events:       <none>\n");
    out
}

fn tree(out: &mut String, title: &str, value: &Value, depth: usize) {
    let pad = "  ".repeat(depth);
    match value {
        Value::Object(map) if !map.is_empty() => {
            let _ = writeln!(out, "{pad}{title}");
            for (key, value) in map {
                tree(out, &format!("{key}:"), value, depth + 1);
            }
        }
        Value::Array(items) if !items.is_empty() => {
            let _ = writeln!(out, "{pad}{title}");
            for item in items {
                tree(out, "-", item, depth + 1);
            }
        }
        Value::Object(_) | Value::Array(_) | Value::Null => {
            let _ = writeln!(out, "{pad}{title:<14}<none>");
        }
        Value::String(s) => {
            let _ = writeln!(out, "{pad}{title:<14}{s}");
        }
        other => {
            let _ = writeln!(out, "{pad}{title:<14}{other}");
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_config_map_lists_its_data() {
        let text = describe(&json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "big", "namespace": "ns", "labels": {"app": "x"}},
            "data": {"a.txt": "line 1\nline 2"},
        }));
        assert!(
            text.starts_with("Name:         big\nNamespace:    ns\n"),
            "{text}"
        );
        assert!(text.contains("Labels:\n  app:          x\n"), "{text}");
        assert!(text.contains("Data\n====\na.txt:\n----\nline 1\nline 2\n"));
        assert!(text.contains("Annotations:  <none>"));
    }
}
