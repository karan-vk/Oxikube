//! `terminal::OpenLink` (E09-S05): what a cmd/ctrl-click on a terminal link runs.
//!
//! The element never opens anything itself: it dispatches `Command::TerminalOpenLink` through the
//! window's command dispatcher, so the palette, the keymap and agents (the
//! `app.terminal_open_link` tool) reach the same handler. [`register_commands`] installs it: the
//! handler validates the target off the UI thread ([`LinkTarget::parse`], then [`plan`]) and
//! hands what to do to the window through a [`LinkSink`]; [`open`] runs it on the UI thread.
//!
//! A process can print any OSC 8 URI, and a pod's process is not trusted, so:
//!
//! * only `http`, `https` and `mailto` URLs go to the browser; other schemes (`javascript:`,
//!   `ssh:`, custom app schemes) are refused rather than handed to the OS;
//! * `file:` URLs and absolute paths must exist; a regular file that is not executable opens with
//!   the system's opener, anything else (a directory, an app bundle, an executable or script) is
//!   only revealed in the file manager, never launched.
//!
//! Nothing here reads or changes a cluster: no `MutationGuard` tier.

use std::path::PathBuf;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use oxikube_app::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::{OxiError, OxiResult};

/// URL schemes the browser may open.
pub const BROWSER_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// A parsed link target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// A URL with a [browser scheme](BROWSER_SCHEMES).
    Url(String),
    /// An absolute local path (from a path or a `file:` URL). A `:line[:column]` suffix is
    /// parsed off (no editor takes it yet).
    Path {
        /// The file or directory.
        path: PathBuf,
        /// The line the link named, if any.
        line: Option<u32>,
        /// The column the link named, if any.
        column: Option<u32>,
    },
}

/// What the window does with a validated link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    /// Open the URL in the browser.
    Browse(String),
    /// Open the file with the system's opener (a regular, non-executable file).
    OpenFile(PathBuf),
    /// Show the item in the file manager without opening it (a directory, a bundle, an
    /// executable).
    Reveal(PathBuf),
}

impl LinkTarget {
    /// Parses a command target: a URL (`scheme:` prefix) or an absolute path with an optional
    /// `:line[:column]` suffix. Does not touch the disk.
    ///
    /// # Errors
    ///
    /// `Validation` for a scheme the app does not open, a malformed URL or a relative path.
    pub fn parse(target: &str) -> OxiResult<Self> {
        let target = target.trim();
        if target.starts_with('/') {
            let (path, line, column) = split_position(target);
            return Ok(Self::Path {
                path: PathBuf::from(path),
                line,
                column,
            });
        }
        let url = url::Url::parse(target)
            .map_err(|_| OxiError::validation("not a URL or an absolute path"))?;
        if url.scheme() == "file" {
            let path = url
                .to_file_path()
                .map_err(|()| OxiError::validation("not a local file URL"))?;
            return Ok(Self::Path {
                path,
                line: None,
                column: None,
            });
        }
        if !BROWSER_SCHEMES.contains(&url.scheme()) {
            return Err(OxiError::validation(format!(
                "links with the `{}` scheme are not opened",
                url.scheme()
            )));
        }
        Ok(Self::Url(url.into()))
    }
}

/// `path:12:3` -> (`path`, 12, 3); a suffix that is not numbers stays part of the path.
fn split_position(target: &str) -> (&str, Option<u32>, Option<u32>) {
    let number = |text: &str| text.parse::<u32>().ok();
    if let Some((rest, column)) = target.rsplit_once(':')
        && let Some(column) = number(column)
    {
        if let Some((path, line)) = rest.rsplit_once(':')
            && let Some(line) = number(line)
        {
            return (path, Some(line), Some(column));
        }
        return (rest, Some(column), None);
    }
    (target, None, None)
}

/// What to do with `target`, reading the file's metadata (call it off the UI thread).
///
/// # Errors
///
/// `NotFound` when a path does not exist.
pub fn plan(target: LinkTarget) -> OxiResult<LinkAction> {
    let path = match target {
        LinkTarget::Url(url) => return Ok(LinkAction::Browse(url)),
        LinkTarget::Path { path, .. } => path,
    };
    let metadata =
        std::fs::metadata(&path).map_err(|_| OxiError::not_found("no such file or directory"))?;
    if metadata.is_file() && !is_executable(&metadata) {
        Ok(LinkAction::OpenFile(path))
    } else {
        Ok(LinkAction::Reveal(path))
    }
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    // Windows decides by extension; until the Windows epic, never launch a file from a link.
    true
}

/// A handle on a window's link queue. Cheap to clone; usable from any thread.
#[derive(Clone, Debug)]
pub struct LinkSink {
    tx: UnboundedSender<LinkAction>,
}

impl LinkSink {
    /// A sink and the receiver the window drains on the UI thread (calling [`open`]).
    pub fn channel() -> (Self, UnboundedReceiver<LinkAction>) {
        let (tx, rx) = unbounded();
        (Self { tx }, rx)
    }
}

/// Registers `terminal::OpenLink` on `registry`: the handler validates the target, decides what
/// to do with it ([`plan`]) and queues that on `sink`. Call it from the binary's command setup:
/// `registry.install("oxikube_terminal", |r| register_commands(r, sink))`.
///
/// # Errors
///
/// A [`RegisterError`] when the id is registered twice (a wiring bug).
pub fn register_commands(
    registry: &mut CommandRegistry,
    sink: LinkSink,
) -> Result<(), RegisterError> {
    let id = CommandId::TERMINAL_OPEN_LINK;
    let meta = *command::lookup(id).ok_or(RegisterError::Undeclared(id))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let sink = sink.clone();
        async move {
            let Command::TerminalOpenLink { target } = command else {
                return Err(OxiError::validation("not a terminal::OpenLink command"));
            };
            let action = plan(LinkTarget::parse(&target)?)?;
            sink.tx
                .unbounded_send(action)
                .map_err(|_| OxiError::internal("the window that opens links is gone"))?;
            Ok(CommandOutput::none())
        }
    })
}

/// Runs a planned link action on the UI thread.
pub fn open(action: &LinkAction, cx: &gpui::App) {
    match action {
        LinkAction::Browse(url) => cx.open_url(url),
        LinkAction::OpenFile(path) => cx.open_with_system(path),
        LinkAction::Reveal(path) => cx.reveal_path(path),
    }
}

#[cfg(test)]
mod tests;
