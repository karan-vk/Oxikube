//! The [`HelmRepoPort`] implementation of [`HelmCli`]: `helm repo add`, `remove` and `update`.
//!
//! These commands change the user's own Helm configuration (`repositories.yaml` and the index
//! cache), never a cluster. Helm keeps the repositories, and whatever credentials were given
//! for them, in that file: this adapter writes nothing of its own.
//!
//! # Credentials
//!
//! A repository password goes to `helm` on stdin (`--password-stdin`), so it is never on the
//! command line (visible in `ps`) and never in a temporary file. A URL that carries credentials
//! (`https://user:pass@host`) is refused for the same reason. Output and error text are redacted
//! with the password blanked first.
//!
//! # Update failures
//!
//! `helm repo update` exits 0 when a repository could not be reached and only prints
//! `...Unable to get an update from the "x" chart repository (url): <reason>`. That is turned into
//! a [`CliFailed`](oxikube_domain::helm::HelmError::CliFailed) error carrying the redacted reason,
//! so a caller sees the failure per repository when it updates them one at a time.

use async_trait::async_trait;
use oxikube_domain::helm::{HelmError, RepoAddRequest};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::HelmRepoPort;

use super::HelmCli;
use super::args;
use super::errors::{Redactor, summary};
use super::run::{Invocation, finish, run};

/// What `helm repo update` prints for a repository it could not refresh.
const UPDATE_FAILED: &str = "Unable to get an update";

#[async_trait]
impl HelmRepoPort for HelmCli {
    async fn repo_add(&self, request: &RepoAddRequest) -> OxiResult<()> {
        let mut request = request.clone();
        request.name = request.name.trim().to_owned();
        request.url = request.url.trim().to_owned();
        let request = &request;
        if has_userinfo(&request.url) {
            return Err(OxiError::validation(
                "a repository URL must not contain credentials",
            ));
        }
        oxikube_domain::helm::validate_repo(&request.name, &request.url)
            .map_err(OxiError::validation)?;
        let password = request.password.as_ref().filter(|p| !p.is_empty());
        let mut invocation = Invocation::new(args::repo_add(request), self.config().timeouts.fetch);
        if let Some(password) = password {
            invocation.stdin = Some(password.expose().to_owned());
            invocation.redactor = Redactor::for_secret(password.expose());
        }
        let (binary, _) = self.binary().await?;
        finish(run(&binary, invocation).await, None).map(drop)
    }

    async fn repo_remove(&self, name: &str) -> OxiResult<()> {
        let read = self.config().timeouts.read;
        self.output(args::repo_remove(name), read, None)
            .await
            .map(drop)
    }

    async fn repo_update(&self, names: &[String]) -> OxiResult<()> {
        let fetch = self.config().timeouts.fetch;
        let out = self.output(args::repo_update(names), fetch, None).await?;
        match failed_update(&out) {
            Some(reason) => Err(HelmError::CliFailed {
                exit_code: Some(0),
                stderr_summary: reason,
            }
            .into()),
            None => Ok(()),
        }
    }
}

/// The redacted reasons of every `Unable to get an update` line, joined, if any.
fn failed_update(stdout: &str) -> Option<String> {
    let mut reasons = Vec::new();
    let mut lines = stdout.lines().map(str::trim).peekable();
    while let Some(line) = lines.next() {
        let Some((_, rest)) = line.split_once(UPDATE_FAILED) else {
            continue;
        };
        // "...Unable to get an update from the "x" chart repository (url):" then the cause on the
        // next line (or on the same one).
        let mut reason = format!("{UPDATE_FAILED}{rest}");
        if let Some(next) = lines.peek()
            && !next.is_empty()
            && !next.contains(UPDATE_FAILED)
            && !next.starts_with("Update Complete")
            && !next.starts_with("Hang tight")
            && !next.starts_with("...")
        {
            reason.push(' ');
            reason.push_str(next);
            lines.next();
        }
        reasons.push(reason);
    }
    if reasons.is_empty() {
        return None;
    }
    Some(summary(
        &format!("Error: {}", reasons.join("; ")),
        &Redactor::none(),
    ))
}

/// `true` when `url` has a `user[:password]@` part before its host.
fn has_userinfo(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority.contains('@')
}
