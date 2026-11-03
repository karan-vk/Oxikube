//! The per-cluster model: the settings page's model over the fields the schema marks as
//! cluster scoped.

use gpui::SharedString;
use oxikube_settings::CLUSTER_SCOPED_KEY;
use serde_json::{Map, Value};

use crate::page::SettingsPageModel;

/// The page model of the per-cluster tab for `schema` (`SettingsStore::json_schema`): only the
/// properties carrying [`CLUSTER_SCOPED_KEY`], the same walk as the global page, so a field
/// added to `ClusterSettingsContent` shows up with no code here. A schema with no such property
/// gives an empty model.
pub fn cluster_page_model(schema: &Value) -> SettingsPageModel {
    let mut model = SettingsPageModel::from_schema(&cluster_schema(schema));
    // The top-level scalars of the global page are "General"; here they are the cluster itself.
    if let Some(section) = model.general_mut() {
        section.title = SharedString::from("Cluster");
        section.description = Some(
            "Name, colour, protection and defaults of the cluster. Each can differ from your global value."
                .into(),
        );
    }
    model
}

/// `schema` with only the cluster-scoped properties (and every definition, for the `$ref`s).
fn cluster_schema(schema: &Value) -> Value {
    let scoped: Map<String, Value> = schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, node)| node.get(CLUSTER_SCOPED_KEY) == Some(&Value::Bool(true)))
        .map(|(key, node)| (key.clone(), node.clone()))
        .collect();
    let mut filtered = Map::new();
    filtered.insert("type".into(), Value::from("object"));
    filtered.insert("properties".into(), Value::Object(scoped));
    if let Some(definitions) = schema.get("$defs") {
        filtered.insert("$defs".into(), definitions.clone());
    }
    Value::Object(filtered)
}

/// Whether the field at the dotted `path` is offered on the per-cluster tab of `schema`.
///
/// A nested path (`prometheus.url`) counts when its top-level key is cluster scoped.
pub fn supports_cluster_scope(schema: &Value, path: &str) -> bool {
    let top = path.split('.').next().unwrap_or(path);
    schema
        .get("properties")
        .and_then(|properties| properties.get(top))
        .and_then(|node| node.get(CLUSTER_SCOPED_KEY))
        == Some(&Value::Bool(true))
}
