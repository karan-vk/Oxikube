//! A failed transfer as a typed error: the program missing, the path missing, no permission.
//!
//! What the remote program printed on stderr is kept as a bounded tail and only used to build
//! the message: it names files, never their content, and it is never logged.

use oxikube_domain::{OxiError, OxiResult};

use super::super::ExitStatus;
use super::super::missing::error_is_missing_program;
use super::RemotePath;

/// The most stderr kept for the message, in bytes (the end of it: that is where the error is).
const TAIL_BYTES: usize = 2048;

/// GNU tar's notice when it strips a leading `/`; harmless, and never the reason for a failure.
const LEADING_SLASH_NOTICE: &str = "Removing leading";

/// What a transfer runs in the container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Program {
    /// `tar -cf -`: download an archive.
    TarCreate,
    /// `tar -xmf -`: upload an archive.
    TarExtract,
    /// `cat`: download one file without tar.
    Cat,
}

impl Program {
    fn name(self) -> &'static str {
        match self {
            Program::TarCreate | Program::TarExtract => "tar",
            Program::Cat => "cat",
        }
    }

    fn missing_advice(self) -> &'static str {
        match self {
            Program::TarCreate => {
                "copying files needs tar and a shell (sh) in the image (a single file can still \
                 be downloaded without them)"
            }
            Program::TarExtract => "uploading files needs tar in the image",
            Program::Cat => "downloading a file needs tar or cat in the image",
        }
    }
}

/// The last [`TAIL_BYTES`] of a stream (stderr).
#[derive(Debug, Default)]
pub(super) struct Tail {
    bytes: Vec<u8>,
}

impl Tail {
    pub(super) fn push(&mut self, chunk: &[u8]) {
        if chunk.len() >= TAIL_BYTES {
            self.bytes.clear();
            self.bytes
                .extend_from_slice(&chunk[chunk.len() - TAIL_BYTES..]);
            return;
        }
        let overflow = (self.bytes.len() + chunk.len()).saturating_sub(TAIL_BYTES);
        self.bytes.drain(..overflow);
        self.bytes.extend_from_slice(chunk);
    }

    /// The kept lines worth showing, joined with `; `.
    pub(super) fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.contains(LEADING_SLASH_NOTICE))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// Who failed: the program and where it ran.
pub(super) struct Failure<'a> {
    pub(super) program: Program,
    pub(super) target: &'a RemotePath,
}

impl Failure<'_> {
    fn unsupported(&self) -> OxiError {
        OxiError::unsupported(format!(
            "{} is not available in {}: {}.",
            self.program.name(),
            self.target.place(),
            self.program.missing_advice()
        ))
    }

    /// A failed open: the runtime refusing a program the image does not have is `Unsupported`;
    /// anything else keeps the adapter's kind and message.
    pub(super) fn open(&self, error: OxiError) -> OxiError {
        if error_is_missing_program(&error) {
            self.unsupported()
        } else {
            error
        }
    }

    /// How the program ended, given what it printed on stderr: `Ok` for exit 0 (whatever the
    /// warnings), else the typed error.
    pub(super) fn exit(&self, status: &ExitStatus, stderr: &Tail) -> OxiResult<()> {
        if status.is_success() {
            return Ok(());
        }
        if status.is_missing_program() {
            return Err(self.unsupported());
        }
        let said = stderr.text();
        let lower = said.to_ascii_lowercase();
        let place = self.target.place();
        let path = &self.target.path;
        if lower.contains("no such file or directory") {
            return Err(OxiError::not_found(format!(
                "{path} was not found in {place}: {said}"
            )));
        }
        if [
            "permission denied",
            "read-only file system",
            "operation not permitted",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
        {
            return Err(OxiError::forbidden(format!(
                "{} could not access {path} in {place}: {said}",
                self.program.name()
            )));
        }
        let how = match (status.code, &status.signal) {
            (Some(code), _) => format!("exit code {code}"),
            (None, Some(signal)) => format!("signal {signal}"),
            (None, None) => status
                .message
                .clone()
                .unwrap_or_else(|| "no exit code".to_owned()),
        };
        let detail = if said.is_empty() {
            String::new()
        } else {
            format!(": {said}")
        };
        Err(OxiError::conflict(format!(
            "{} failed in {place} ({how}){detail}",
            self.program.name()
        )))
    }
}
