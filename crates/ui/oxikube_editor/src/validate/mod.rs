//! Schema validation of manifests (E10-S03): a document of the spanned YAML model checked
//! against the cluster's [`JsonSchema`], producing [`Diagnostic`]s the editor draws as squiggles
//! (E10-S04), and hover and completion (E10-S05) share.
//!
//! Pure and synchronous: no gpui, no kube, no `SchemaPort` call. The caller fetches the schema
//! (`oxikube_runtime::spawn_kube`, never on the UI thread), parses the buffer once
//! ([`yaml::ParseCache`](crate::yaml::ParseCache)) and runs [`validate`] or [`validate_buffer`]
//! on the background executor, dropping the result if the buffer has moved on.
//!
//! # Rules
//!
//! | Code | Severity | Meaning |
//! |---|---|---|
//! | `syntax` | error | broken YAML (from the model, added by [`validate_buffer`]) |
//! | `type-mismatch` | error | the value's YAML 1.2 core type is not one the schema allows |
//! | `enum` | error | not one of the schema's `enum` values (with "did you mean") |
//! | `required` | error | a `required` property is missing |
//! | `pattern` | error | a string does not match the schema's `pattern` |
//! | `unknown-field` | warning | a key the schema does not list (with "did you mean") |
//! | `duplicate-key` | warning | `x-kubernetes-list-type: map` items sharing their list-map keys |
//! | `duplicate-item` | warning | `x-kubernetes-list-type: set` scalars repeated |
//!
//! Typing follows the YAML 1.2 core schema, as the API server sees the manifest: `replicas: "3"`
//! is a string and `replicas: 3` an integer; `yes` is a string; `500m`, `1Gi` and `5s` are
//! strings, so an int-or-string or quantity field takes them. An empty value (`key:`) is a null,
//! which the server treats as "not set", and is never reported. `3.0` is accepted for an integer.
//! Format checks beyond `pattern` (`int32` range, `date-time`) are out of scope.
//!
//! Unknown fields: Kubernetes schemas are structural, so an object that lists `properties` and
//! does not carry `x-kubernetes-preserve-unknown-fields` rejects other keys (as does an explicit
//! `additionalProperties: false`). An object that lists none is free-form, and one whose
//! `additionalProperties` is a schema validates every other value against it. Subtrees the
//! schema flattening cut off ([`JsonSchema::truncated`]) are not judged. The root `status` is
//! skipped by default ([`ValidateOptions::skip_status`]).

mod diagnostic;
mod list;
mod object;
mod options;
mod run;
mod scalar;
mod suggest;
mod walk;

pub use diagnostic::{Diagnostic, DiagnosticCode, Severity};
pub use options::ValidateOptions;
pub use run::{document_gvk, validate, validate_buffer};

#[cfg(doc)]
use oxikube_domain::schema::JsonSchema;
