//! Listing a directory of a container: `ls -la`, else `find`.
//!
//! One `sh` script per attempt, run without a TTY. `LC_ALL=C` fixes the locale (English month
//! names, no localised columns). `--full-time` is used when the container's `ls` accepts it
//! (probed by running it once on `/`), so times are exact and carry their zone; otherwise the
//! classic columns are read. A container whose `ls` is missing gets the `find` script (names and
//! kinds only); one with neither, or no `sh`, gets the no-shell-tools error that points to a
//! debug container.

use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{ExecStreamPort, ExitStatus};

use super::capture::{Captured, capture};
use super::entry::{Listing, ListingSource, directory_argument};
use super::parse::{parse_find, parse_ls};

const LS_SCRIPT: &str = r#"LC_ALL=C; export LC_ALL
if ls --full-time -d / >/dev/null 2>&1; then ls -la --full-time -- "$1"; else ls -la -- "$1"; fi"#;

const FIND_SCRIPT: &str = r#"LC_ALL=C; export LC_ALL
find "$1" -mindepth 1 -maxdepth 1 -exec sh -c 'for f; do if [ -L "$f" ]; then printf "l %s\n" "$f"; elif [ -d "$f" ]; then printf "d %s\n" "$f"; else printf "f %s\n" "$f"; fi; done' sh {} +"#;

/// What [`FileService::list`](super::FileService::list) runs: a script with the directory as `$1`.
fn script_command(script: &str, directory: &str) -> Vec<String> {
    ["sh", "-c", script, "sh", directory]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// The advice shown in place of a listing when the container has no tools to list with.
pub const NO_SHELL_TOOLS: &str = "This container has no shell tools (sh, ls or find), so its \
     files cannot be listed. It may be a distroless or scratch image: open a debug container \
     to look inside it.";

fn no_shell_tools() -> OxiError {
    OxiError::unsupported(NO_SHELL_TOOLS)
}

/// Lists `path` in `container` of `namespace/pod`. `now` (Unix seconds) dates entries that have no
/// year.
///
/// # Errors
///
/// `Unsupported` ([`NO_SHELL_TOOLS`]) when the container has no `sh`, or neither `ls` nor `find`;
/// `NotFound` for a missing directory; `Forbidden` for one the container's user may not read;
/// `Validation` for a path that is not a directory; `Timeout`, and the open's own errors.
pub(super) async fn list_directory(
    port: &dyn ExecStreamPort,
    namespace: &str,
    pod: &str,
    container: Option<&str>,
    path: &str,
    now: i64,
) -> OxiResult<Listing> {
    if path.is_empty() || path.contains('\0') {
        return Err(OxiError::validation("a directory path is needed"));
    }
    let directory = directory_argument(path);
    let run = |script: &'static str| {
        let command = script_command(script, &directory);
        async move {
            capture(port, namespace, pod, container, &command)
                .await
                .map_err(|error| {
                    if oxikube_ports::exec::error_is_missing_program(&error) {
                        no_shell_tools()
                    } else {
                        error
                    }
                })
        }
    };
    let ls = run(LS_SCRIPT).await?;
    if missing_program(&ls) {
        return list_with_find(&run, path).await;
    }
    let stdout = String::from_utf8_lossy(&ls.stdout);
    let parsed = parse_ls(&stdout, now);
    if !ls.status.is_success() && parsed.entries.is_empty() {
        return Err(failure(&ls, path));
    }
    Ok(Listing {
        path: path.to_owned(),
        entries: parsed.entries,
        unreadable: parsed.unreadable,
        source: ListingSource::Ls,
    })
}

async fn list_with_find<F, Fut>(run: &F, path: &str) -> OxiResult<Listing>
where
    F: Fn(&'static str) -> Fut,
    Fut: std::future::Future<Output = OxiResult<Captured>>,
{
    let found = run(FIND_SCRIPT).await?;
    if missing_program(&found) {
        return Err(no_shell_tools());
    }
    if !found.status.is_success() && found.stdout.is_empty() {
        return Err(failure(&found, path));
    }
    Ok(Listing {
        path: path.to_owned(),
        entries: parse_find(&String::from_utf8_lossy(&found.stdout), path),
        unreadable: 0,
        source: ListingSource::Find,
    })
}

/// The script ran but a program in it was not found: `sh` reports `ls: not found` (busybox) or
/// `ls: command not found` with 127.
fn missing_program(captured: &Captured) -> bool {
    captured.status.is_missing_program() && captured.stdout.is_empty()
}

/// A listing that failed, as a typed error from what `ls` said (names only, never contents).
fn failure(captured: &Captured, path: &str) -> OxiError {
    let said = String::from_utf8_lossy(&captured.stderr);
    let lower = said.to_ascii_lowercase();
    let line = said
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if lower.contains("no such file or directory") {
        OxiError::not_found(format!("{path} was not found in the container"))
    } else if lower.contains("permission denied") {
        OxiError::forbidden(format!("the container's user may not read {path}"))
    } else if lower.contains("not a directory") {
        OxiError::validation(format!("{path} is not a directory"))
    } else {
        OxiError::conflict(format!(
            "listing {path} failed ({}){}",
            exit_words(&captured.status),
            if line.is_empty() {
                String::new()
            } else {
                format!(": {line}")
            }
        ))
    }
}

fn exit_words(status: &ExitStatus) -> String {
    match (status.code, &status.signal) {
        (Some(code), _) => format!("exit code {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        (None, None) => "no exit code".to_owned(),
    }
}
