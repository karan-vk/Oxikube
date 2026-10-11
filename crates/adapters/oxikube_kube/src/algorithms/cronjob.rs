// Portions derived from kdash (https://github.com/kdash-rs/kdash), `src/network/mod.rs` at commit
// c303673 (v2.1.1): `trigger_cronjob` (clone `spec.jobTemplate` into a `Job` with a generated
// name and an owner reference to the CronJob; mirrors `kubectl create job --from=cronjob/x`).
// MIT licence; the full text follows. Modifications (c) Oxikube contributors: the algorithm runs
// on a `ResourcePort` instead of a kube `Api`, returns the created `Job`, takes the write options
// (dry run, field manager), truncates the generated-name prefix to what the server keeps, keeps the
// job template's labels and annotations and adds kubectl's `cronjob.kubernetes.io/instantiate`
// annotation, and is pinned by exact-JSON tests. The manifest builder moved to
// `oxikube_domain::build` (E12-S08) so the app builds the same Job; this file keeps the create.
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
//! The manifest is [`oxikube_domain::build::job_from_cronjob`] (the template verbatim, a
//! `generateName: <cronjob>-manual-`, kubectl's instantiate annotation and an owner reference to
//! the CronJob); this function reads the CronJob and creates the Job.

use oxikube_domain::build::job_from_cronjob;
use oxikube_domain::{OxiResult, Resource};
use oxikube_ports::{ResourcePort, WriteOptions};
use tracing::debug;

use super::api::{cronjob_gvk, job_gvk};

pub use oxikube_domain::build::INSTANTIATE_ANNOTATION;

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
