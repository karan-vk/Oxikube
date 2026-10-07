//! The mapped keystrokes against a real PTY (the Linux and macOS smoke test of E09-S06): what
//! `to_esc_str` produces is what a terminal program actually receives, including the line
//! discipline's own reading of it (`ctrl-c` becomes SIGINT).
#![cfg(unix)]

use std::time::Duration;

use futures::StreamExt as _;
use gpui::Keystroke;
use oxikube_ports::exec::{BackendEvent, TerminalBackend as _};
use oxikube_terminal::backend::local::{LocalPty, LocalPtyOptions};
use oxikube_terminal::mappings::{KeyMode, encode_paste, to_esc_str};

const WAIT: Duration = Duration::from_secs(20);

fn sh(script: &str) -> LocalPty {
    LocalPty::spawn(LocalPtyOptions {
        shell: Some("/bin/sh".into()),
        args: vec!["-c".into(), script.into()],
        ..LocalPtyOptions::default()
    })
    .expect("spawn /bin/sh")
}

fn keys(text: &str) -> Vec<u8> {
    let keystroke = Keystroke::parse(text).expect("a keystroke");
    to_esc_str(&keystroke, KeyMode::default())
        .expect("a terminal key")
        .as_bytes()
        .to_vec()
}

/// Reads output until `done` holds or the process exits; returns the output and the exit.
async fn read_until(
    stream: &mut futures::stream::BoxStream<'static, BackendEvent>,
    done: impl Fn(&str) -> bool,
) -> (String, Option<oxikube_ports::exec::ExitStatus>) {
    let read = async {
        let mut output = String::new();
        while let Some(event) = stream.next().await {
            match event {
                BackendEvent::Output(bytes) => {
                    output.push_str(&String::from_utf8_lossy(&bytes));
                    if done(&output) {
                        return (output, None);
                    }
                }
                BackendEvent::Exited(status) => return (output, Some(status)),
                BackendEvent::Error(error) => panic!("backend error: {error}"),
            }
        }
        (output, None)
    };
    tokio::time::timeout(WAIT, read)
        .await
        .expect("the program answered in time")
}

#[tokio::test]
async fn ctrl_c_reaches_the_pty_as_an_interrupt() {
    let pty = sh("sleep 30");
    let mut stream = pty.output_stream();
    // Let the shell start `sleep` before interrupting it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    pty.write(&keys("ctrl-c")).await.unwrap();
    let (_, status) = read_until(&mut stream, |_| false).await;
    let status = status.expect("ctrl-c ended the process");
    assert!(
        status.signal.as_deref() == Some("INT") || status.code == Some(130),
        "{status:?}"
    );
}

#[tokio::test]
async fn typed_text_and_enter_reach_a_program() {
    let pty = sh("cat");
    let mut stream = pty.output_stream();
    pty.write("héllo".as_bytes()).await.unwrap();
    pty.write(&keys("enter")).await.unwrap();
    // The tty echoes the line and cat writes it again.
    let (output, _) = read_until(&mut stream, |out| out.matches("héllo").count() >= 2).await;
    assert!(output.matches("héllo").count() >= 2, "{output:?}");
    pty.write(&keys("ctrl-d")).await.unwrap();
    let (_, status) = read_until(&mut stream, |_| false).await;
    assert!(status.is_some_and(|status| status.is_success()));
}

#[tokio::test]
async fn arrow_keys_arrive_as_escape_sequences() {
    // `cat -v` shows control characters: ESC [ A is `^[[A`.
    let pty = sh("cat -v");
    let mut stream = pty.output_stream();
    pty.write(&keys("up")).await.unwrap();
    pty.write(&keys("ctrl-right")).await.unwrap();
    pty.write(b"\n").await.unwrap();
    let (output, _) = read_until(&mut stream, |out| {
        out.contains("^[[A^[[1;5C") && out.matches('\n').count() >= 2
    })
    .await;
    assert!(output.contains("^[[A^[[1;5C"), "{output:?}");
    pty.write(&keys("ctrl-d")).await.unwrap();
}

#[tokio::test]
async fn a_pasted_block_arrives_whole() {
    let pty = sh("cat -v");
    let mut stream = pty.output_stream();
    pty.write(&encode_paste("one\ntwo\n", true)).await.unwrap();
    // Bracketed: the markers are visible to a program that did not ask for them.
    let (output, _) = read_until(&mut stream, |out| out.contains("^[[201~")).await;
    assert!(output.contains("^[[200~one"), "{output:?}");
    assert!(output.contains("two"), "{output:?}");
    pty.write(&keys("ctrl-d")).await.unwrap();
}
