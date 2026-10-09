// Portions derived from kdash (https://github.com/kdash-rs/kdash), `src/network/mod.rs` at commit
// c303673 (v2.1.1): `trigger_cronjob` (clone `spec.jobTemplate` into a `Job` with a generated
// name and an owner reference to the CronJob; mirrors `kubectl create job --from=cronjob/x`).
// MIT licence; the full text follows. Modifications (c) Oxikube contributors: the algorithm runs
// on a `ResourcePort` instead of a kube `Api`, returns the created `Job`, takes the write options
// (dry run, field manager), truncates the generated-name prefix to what the server keeps, keeps the
// job template's labels and annotations and adds kubectl's `cronjob.kubernetes.io/instantiate`
// annotation, and is pinned by exact-JSON tests.
//
// Copyright (c) 2021 Deepu K Sasidharan
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

//! `trigger_cronjob`: run a CronJob now by creating a Job from its template.
//!
//! The Job is `spec.jobTemplate` verbatim (its `spec`, labels and annotations), named by the
//! API server from `generateName: <cronjob>-manual-`, annotated
//! [`cronjob.kubernetes.io/instantiate: manual`](INSTANTIATE_ANNOTATION) as kubectl does and
//! owned by the CronJob (`controller` and `blockOwnerDeletion` true), so deleting the CronJob
//! deletes the Jobs it triggered and the CronJob's history limits apply to them.

use oxikube_domain::json::JsonRef;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{ResourcePort, WriteOptions};
use serde_json::{Map, Value, json};
use tracing::debug;

use super::api::{cronjob_gvk, job_gvk};

/// The annotation `kubectl create job --from=cronjob/x` puts on the Jobs it creates.
pub const INSTANTIATE_ANNOTATION: &str = "cronjob.kubernetes.io/instantiate";

/// The most characters of `generateName` the API server keeps (63 minus its 5 random ones).
const MAX_GENERATE_NAME: usize = 58;

/// What follows the CronJob's name in the generated name.
const SUFFIX: &str = "-manual-";

/// Creates a Job from the CronJob `namespace/name`'s job template and returns it.
///
/// With `options.dry_run` the server validates and returns the Job it would create (with a
/// generated name) and stores nothing.
///
/// # Errors
///
/// `NotFound` for a missing CronJob, `Validation` if it has no `spec.jobTemplate.spec` (or no
/// uid for the owner reference), and the port's errors for the create (`Forbidden`, `Validation` from admission, ...).
pub async fn trigger_cronjob(
    port: &dyn ResourcePort,
    namespace: &str,
    name: &str,
    options: &WriteOptions,
) -> OxiResult<Resource> {
    let cronjob = port.get(&cronjob_gvk(), Some(namespace), name).await?;
    let job = job_from_cronjob(&cronjob)?;
    debug!(
        op = "trigger_cronjob",
        namespace,
        name,
        dry_run = options.dry_run,
        "algorithm"
    );
    port.create(&job_gvk(), Some(namespace), &job, options)
        .await
}

/// The Job manifest `kubectl create job --from` builds for `cronjob`.
///
/// Pure: no clock and no randomness (the server generates the name suffix).
///
/// # Errors
///
/// `Validation` if the CronJob has no `spec.jobTemplate.spec`, no uid, or its template labels or
/// annotations are not objects.
pub fn job_from_cronjob(cronjob: &Resource) -> OxiResult<Value> {
    // A user action on one CronJob, so decoding its template to a tree is fine.
    let template_value = cronjob.get("/spec/jobTemplate").map(JsonRef::to_value);
    let template = template_value.as_ref().and_then(Value::as_object);
    let Some(spec) = template
        .and_then(|t| t.get("spec"))
        .filter(|s| s.is_object())
    else {
        return Err(OxiError::validation(format!(
            "cronjob {} has no job template",
            cronjob.name()
        )));
    };
    let Some(uid) = cronjob.meta.uid.as_deref() else {
        return Err(OxiError::validation(format!(
            "cronjob {} has no uid to own the job",
            cronjob.name()
        )));
    };
    let template_meta = template.and_then(|t| t.get("metadata"));
    let string_map = |key: &str| -> OxiResult<Map<String, Value>> {
        match template_meta.and_then(|m| m.get(key)) {
            None | Some(Value::Null) => Ok(Map::new()),
            Some(Value::Object(map)) => Ok(map.clone()),
            Some(_) => Err(OxiError::validation(format!(
                "cronjob {} has a job template with invalid {key}",
                cronjob.name()
            ))),
        }
    };
    let mut annotations = string_map("annotations")?;
    annotations.insert(INSTANTIATE_ANNOTATION.to_owned(), json!("manual"));
    let mut metadata = json!({
        "generateName": generate_name(cronjob.name()),
        "annotations": annotations,
        "ownerReferences": [{
            "apiVersion": "batch/v1",
            "kind": "CronJob",
            "name": cronjob.name(),
            "uid": uid,
            "controller": true,
            "blockOwnerDeletion": true,
        }],
    });
    if let Some(namespace) = cronjob.namespace() {
        metadata["namespace"] = json!(namespace);
    }
    let labels = string_map("labels")?;
    if !labels.is_empty() {
        metadata["labels"] = Value::Object(labels);
    }
    Ok(json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": metadata,
        "spec": spec,
    }))
}

/// `<cronjob>-manual-`, with the CronJob's name cut so the whole prefix survives the server's
/// own truncation (a 52-character CronJob name would otherwise lose `-manual-`).
fn generate_name(cronjob: &str) -> String {
    let keep = MAX_GENERATE_NAME - SUFFIX.len();
    let base: String = cronjob.chars().take(keep).collect();
    format!("{base}{SUFFIX}")
}
