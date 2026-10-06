//! A real shell on a real PTY: output, exit codes, input, resize.

use oxikube_ports::exec::TerminalSize;

use super::*;

#[tokio::test]
async fn output_and_the_exit_code_arrive_and_the_stream_ends() {
    let pty = LocalPty::spawn(sh("echo hi; exit 3")).unwrap();
    let run = read_to_exit(&pty).await;
    assert!(run.output.contains("hi"), "{:?}", run.output);
    assert_eq!(run.status, Some(ExitStatus::with_code(3)));
    assert!(run.ended, "nothing follows the exit");
}

#[tokio::test]
async fn a_second_output_stream_is_empty() {
    let pty = LocalPty::spawn(sh("exit 0")).unwrap();
    let mut first = pty.output_stream();
    assert!(pty.output_stream().next().await.is_none());
    let run = read_until(&mut first, |_| false).await;
    assert!(run.status.unwrap().is_success());
}

#[tokio::test]
async fn written_bytes_reach_the_process_and_echo_back() {
    let pty = LocalPty::spawn(sh("cat")).unwrap();
    let mut stream = pty.output_stream();
    pty.write(b"hello oxikube\n").await.unwrap();
    // The tty echoes the input, and `cat` writes it again.
    let run = read_until(&mut stream, |out| out.matches("hello oxikube").count() >= 2).await;
    assert!(
        run.output.matches("hello oxikube").count() >= 2,
        "{:?}",
        run.output
    );
    // ^D ends cat: a clean exit.
    pty.write(&[0x04]).await.unwrap();
    let run = read_until(&mut stream, |_| false).await;
    assert_eq!(run.status, Some(ExitStatus::success()));
}

#[tokio::test]
async fn resize_changes_what_the_shell_sees() {
    let pty = LocalPty::spawn(sh("stty size; read _; stty size")).unwrap();
    let mut stream = pty.output_stream();
    let first = read_until(&mut stream, |out| out.contains("24 80")).await;
    assert!(first.output.contains("24 80"), "{:?}", first.output);

    pty.resize(TerminalSize::new(100, 30).with_pixels(800, 600))
        .await
        .unwrap();
    pty.write(b"\n").await.unwrap();
    let second = read_until(&mut stream, |out| out.contains("30 100")).await;
    assert!(second.output.contains("30 100"), "{:?}", second.output);
}

#[tokio::test]
async fn the_working_directory_is_honoured() {
    let dir = tempfile::tempdir().unwrap();
    let canonical = dir.path().canonicalize().unwrap();
    let pty = LocalPty::spawn(sh("pwd -P").in_dir(&canonical)).unwrap();
    let run = read_to_exit(&pty).await;
    assert!(
        run.output.contains(canonical.to_str().unwrap()),
        "{:?}",
        run.output
    );
}

#[tokio::test]
async fn output_bytes_are_passed_through_untouched() {
    let pty = LocalPty::spawn(sh(r"printf '\033[31mred\033[0m\303\251'")).unwrap();
    let mut stream = pty.output_stream();
    let mut bytes = Vec::new();
    while let Some(event) = stream.next().await {
        if let BackendEvent::Output(chunk) = event {
            bytes.extend_from_slice(&chunk);
        }
    }
    assert_eq!(bytes, b"\x1b[31mred\x1b[0m\xc3\xa9");
}
