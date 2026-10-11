//! Shared helpers of the E10-S05 hover and completion tests.

#![allow(dead_code, reason = "each test binary uses a subset")]

use std::sync::Arc;

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::{JsonSchema, root_schema_for};
use oxikube_editor::assist::{Buffer, CompletionItem, HoverDoc, complete_at, hover_at};
use oxikube_editor::yaml::{ParseResult, parse};
use oxikube_testkit::fixtures::openapi;
use serde_json::json;

/// A Pod schema with the parts of the real one hover and completion are about: an enum on
/// `restartPolicy` and `imagePullPolicy`, a boolean, a defaulted enum, map-typed labels, lists
/// of containers, ports, env and volumes. Written as an OpenAPI document so `$ref` and `allOf`
/// flattening is part of what is tested.
pub fn pod_document() -> serde_json::Value {
    let schemas = "#/components/schemas";
    json!({"components": {"schemas": {
        "Pod": {
            "description": "Pod is a collection of containers that can run on a host.",
            "type": "object",
            "properties": {
                "apiVersion": {"type": "string", "description": "APIVersion defines the versioned schema."},
                "kind": {"type": "string", "description": "Kind is a string value representing the REST resource."},
                "metadata": {"allOf": [{"$ref": format!("{schemas}/ObjectMeta")}], "description": "Standard object metadata."},
                "spec": {"allOf": [{"$ref": format!("{schemas}/PodSpec")}], "description": "Specification of the desired behavior of the pod."},
                "status": {"type": "object", "description": "Most recently observed status."},
            },
            "x-kubernetes-group-version-kind": [{"group": "", "kind": "Pod", "version": "v1"}],
        },
        "ObjectMeta": {"type": "object", "properties": {
            "name": {"type": "string", "description": "Name must be unique within a namespace."},
            "namespace": {"type": "string", "description": "Namespace defines the space within which each name is unique."},
            "labels": {"type": "object", "description": "Map of string keys and values used to organize objects.",
                "additionalProperties": {"type": "string"}},
        }},
        "PodSpec": {
            "description": "PodSpec is a description of a pod.",
            "type": "object",
            "required": ["containers"],
            "properties": {
                "containers": {
                    "description": "List of containers belonging to the pod.",
                    "type": "array",
                    "items": {"$ref": format!("{schemas}/Container")},
                    "x-kubernetes-list-type": "map",
                    "x-kubernetes-list-map-keys": ["name"],
                },
                "initContainers": {"type": "array", "items": {"$ref": format!("{schemas}/Container")}},
                "restartPolicy": {
                    "type": "string",
                    "description": "Restart policy for all containers within the pod.",
                    "enum": ["Always", "OnFailure", "Never"],
                    "default": "Always",
                },
                "hostNetwork": {"type": "boolean", "description": "Host networking requested for this pod."},
                "volumes": {"type": "array", "items": {"$ref": format!("{schemas}/Volume")}},
                "nodeSelector": {"type": "object", "additionalProperties": {"type": "string"}},
            },
        },
        "Container": {
            "description": "A single application container that you want to run within a pod.",
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"type": "string", "description": "Name of the container specified as a DNS_LABEL."},
                "image": {"type": "string", "description": "Container image name."},
                "imagePullPolicy": {
                    "type": "string",
                    "description": "Image pull policy.",
                    "enum": ["Always", "IfNotPresent", "Never"],
                },
                "command": {"type": "array", "items": {"type": "string"}, "description": "Entrypoint array."},
                "ports": {"type": "array", "items": {"$ref": format!("{schemas}/ContainerPort")}},
                "resources": {"type": "object", "properties": {
                    "limits": {"type": "object", "additionalProperties": {"type": "string"}},
                    "requests": {"type": "object", "additionalProperties": {"type": "string"}},
                }},
                "stdin": {"type": "boolean"},
            },
        },
        "ContainerPort": {"type": "object", "required": ["containerPort"], "properties": {
            "containerPort": {"type": "integer", "format": "int32", "description": "Number of port to expose on the pod's IP address."},
            "protocol": {"type": "string", "default": "TCP", "enum": ["SCTP", "TCP", "UDP"],
                "description": "Protocol for port."},
        }},
        "Volume": {"type": "object", "required": ["name"], "properties": {
            "name": {"type": "string", "description": "Volume's name."},
            "emptyDir": {"type": "object", "description": "A temporary directory."},
            "configMap": {"type": "object", "properties": {"name": {"type": "string"}}},
            "persistentVolumeClaim": {"type": "object", "properties": {"claimName": {"type": "string"}}},
        }},
    }}})
}

pub fn pod_schema() -> Arc<JsonSchema> {
    Arc::new(root_schema_for(&pod_document(), &Gvk::new("", "v1", "Pod")).expect("pod"))
}

pub fn widget_schema() -> Arc<JsonSchema> {
    Arc::new(
        root_schema_for(
            &openapi::widget_crd(),
            &Gvk::new("example.com", "v1", "Widget"),
        )
        .expect("widget"),
    )
}

pub fn deployment_schema() -> Arc<JsonSchema> {
    Arc::new(
        root_schema_for(
            &openapi::deployment_apps_v1(),
            &Gvk::new("apps", "v1", "Deployment"),
        )
        .expect("deployment"),
    )
}

/// The schemas the tests know, by kind.
pub fn schemas(gvk: &Gvk) -> Option<Arc<JsonSchema>> {
    match &*gvk.kind {
        "Pod" => Some(pod_schema()),
        "Deployment" => Some(deployment_schema()),
        "Widget" => Some(widget_schema()),
        _ => None,
    }
}

/// `marked` with the `‸` removed, and where it stood.
pub fn cursor(marked: &str) -> (String, usize) {
    let at = marked.find('‸').expect("a ‸ marks the cursor");
    (marked.replacen('‸', "", 1), at)
}

/// The hover at the `‸` of `marked`.
pub fn hover(marked: &str) -> Option<HoverDoc> {
    let (text, at) = cursor(marked);
    let parsed: ParseResult = parse(&text);
    hover_at(&parsed, schemas, at)
}

/// The completions at the `‸` of `marked`, and the text without it.
pub fn complete(marked: &str) -> (String, Vec<CompletionItem>) {
    let (text, at) = cursor(marked);
    let items = complete_at(&Buffer::new(text.as_str()), schemas, at);
    (text, items)
}

pub fn labels(items: &[CompletionItem]) -> Vec<&str> {
    items.iter().map(|i| i.label.as_str()).collect()
}

/// `text` with `item` applied: its range replaced by its insert text.
pub fn apply(text: &str, item: &CompletionItem) -> String {
    format!(
        "{}{}{}",
        &text[..item.replace.start],
        item.insert_text,
        &text[item.replace.end..]
    )
}

pub fn item<'a>(items: &'a [CompletionItem], label: &str) -> &'a CompletionItem {
    items
        .iter()
        .find(|i| i.label == label)
        .unwrap_or_else(|| panic!("no {label} in {:?}", labels(items)))
}

pub const POD: &str = "\
apiVersion: v1
kind: Pod
metadata:
  name: web
  labels:
    app: web
spec:
  restartPolicy: Always
  containers:
    - name: nginx
      image: nginx:1.27
      imagePullPolicy: IfNotPresent
      ports:
        - containerPort: 8080
          protocol: TCP
    - name: sidecar
      image: busybox
";
