//! Tests of [`LocalPty`]. The PTY ones run real `/bin/sh` and `cat` processes, so they are
//! gated to Unix (macOS and Linux); the options tests run everywhere.

mod env;
#[cfg(unix)]
mod lifecycle;
#[cfg(unix)]
mod session;

use std::time::Duration;

use futures::stream::BoxStream;
use oxikube_ports::exec::{BackendEvent, ExitStatus};

use super::*;

/// Long enough for a loaded CI machine, short enough to fail a hang.
pub(super) const WAIT: Duration = Duration::from_secs(20);

/// A `/bin/sh -c script` terminal.
pub(super) fn sh(script: &str) -> LocalPtyOptions {
    LocalPtyOptions {
        shell: Some("/bin/sh".into()),
        args: vec!["-c".into(), script.into()],
        ..LocalPtyOptions::default()
    }
}

/// The output so far and the status, once the stream reported the exit.
pub(super) struct Run {
    pub(super) output: String,
    pub(super) status: Option<ExitStatus>,
    pub(super) ended: bool,
}

/// Reads `stream` until `done(output)` holds, the exit arrives or the time is up.
pub(super) async fn read_until(
    stream: &mut BoxStream<'static, BackendEvent>,
    mut done: impl FnMut(&str) -> bool,
) -> Run {
    let mut run = Run {
        output: String::new(),
        status: None,
        ended: false,
    };
    let result = tokio::time::timeout(WAIT, async {
        while let Some(event) = stream.next().await {
            match event {
                BackendEvent::Output(bytes) => {
                    run.output.push_str(&String::from_utf8_lossy(&bytes));
                    if done(&run.output) {
                        return;
                    }
                }
                BackendEvent::Exited(status) => {
                    run.status = Some(status);
                    // The stream must end right after the exit.
                    run.ended = stream.next().await.is_none();
                    return;
                }
                BackendEvent::Error(error) => panic!("backend error: {error}"),
            }
        }
        run.ended = true;
    })
    .await;
    assert!(result.is_ok(), "timed out; output so far: {:?}", run.output);
    run
}

/// Reads to the end of the session.
pub(super) async fn read_to_exit(pty: &LocalPty) -> Run {
    read_until(&mut pty.output_stream(), |_| false).await
}

#[test]
fn the_shell_is_the_setting_then_shell_then_bin_sh() {
    assert_eq!(
        resolve_shell(Some("/bin/zsh"), Some("/bin/bash")),
        "/bin/zsh"
    );
    assert_eq!(resolve_shell(None, Some("/bin/bash")), "/bin/bash");
    assert_eq!(resolve_shell(Some("  "), Some(" /bin/bash ")), "/bin/bash");
    assert_eq!(resolve_shell(Some(""), Some("")), "/bin/sh");
    assert_eq!(resolve_shell(None, None), "/bin/sh");
}

#[test]
fn the_settings_become_options() {
    let settings = crate::TerminalSettings {
        shell: Some("/usr/bin/fish".into()),
        shell_args: vec!["-l".into()],
    };
    let options = LocalPtyOptions::from_settings(&settings);
    assert_eq!(options.resolved_shell(), "/usr/bin/fish");
    assert_eq!(options.args, ["-l"]);
    assert_eq!(options.size, DEFAULT_SIZE);
}
