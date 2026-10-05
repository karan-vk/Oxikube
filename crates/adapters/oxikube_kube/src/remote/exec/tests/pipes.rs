//! The stdin sink, the output chunk stream and the resize sink.

use futures::channel::mpsc;
use futures::{FutureExt, SinkExt, StreamExt};
use kube::api::TerminalSize as KubeSize;
use oxikube_domain::ErrorKind;
use oxikube_ports::TerminalSize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

use crate::remote::exec::output::chunks;
use crate::remote::exec::params::STREAM_BUFFER;
use crate::remote::exec::resize::resize_sink;
use crate::remote::exec::stdin::StdinWriter;

#[tokio::test]
async fn stdin_chunks_arrive_in_order_and_closing_ends_the_input() {
    let (writer, mut reader) = tokio::io::duplex(1024);
    let mut sink = StdinWriter::new(writer);
    sink.send(b"echo ".to_vec()).await.expect("send");
    sink.send(b"hello\n".to_vec()).await.expect("send");
    sink.close().await.expect("close");
    let mut received = Vec::new();
    reader.read_to_end(&mut received).await.expect("read");
    assert_eq!(received, b"echo hello\n", "EOF only after the sink closed");
    // Closing twice and flushing a closed sink are not errors.
    sink.close().await.expect("close again");
    sink.flush().await.expect("flush");
}

#[tokio::test]
async fn stdin_waits_for_a_slow_reader_instead_of_buffering() {
    let (writer, mut reader) = tokio::io::duplex(4);
    let mut sink = StdinWriter::new(writer);
    // 4 bytes fit in the pipe, the next chunk has to wait for the reader.
    sink.send(b"abcd".to_vec()).await.expect("first");
    let second = sink.send(b"efgh".to_vec());
    let mut second = Box::pin(second);
    assert!(second.as_mut().now_or_never().is_none(), "backpressure");
    let mut four = [0u8; 4];
    reader.read_exact(&mut four).await.expect("drain");
    assert_eq!(&four, b"abcd");
    second
        .await
        .expect("second goes through once there is room");
    sink.close().await.expect("close");
    let mut rest = Vec::new();
    reader.read_to_end(&mut rest).await.expect("rest");
    assert_eq!(rest, b"efgh");
}

#[tokio::test]
async fn stdin_after_the_process_ended_is_a_network_error() {
    let (writer, reader) = tokio::io::duplex(16);
    let mut sink = StdinWriter::new(writer);
    drop(reader);
    let err = sink.send(b"x".to_vec()).await.expect_err("nobody reads");
    assert_eq!(err.kind(), ErrorKind::Network);
    let err = sink.send(b"y".to_vec()).await.expect_err("still closed");
    assert_eq!(err.kind(), ErrorKind::Network);
}

#[tokio::test]
async fn output_passes_everything_through_and_ends_when_the_pipe_closes() {
    let (mut writer, reader) = tokio::io::duplex(STREAM_BUFFER);
    let payload: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let sent = payload.clone();
    tokio::spawn(async move {
        writer.write_all(&sent).await.expect("write");
        // Dropping the writer is the end of the stream.
    });
    let mut stream = chunks(reader);
    let mut received = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("chunk");
        assert!(!chunk.is_empty() && chunk.len() <= STREAM_BUFFER);
        received.extend_from_slice(&chunk);
    }
    assert_eq!(received, payload);
    assert!(stream.next().await.is_none(), "stays ended");
}

/// A reader that fails on the first read.
struct Broken;

impl AsyncRead for Broken {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Err(std::io::Error::other("reset")))
    }
}

#[tokio::test]
async fn an_output_read_error_is_one_network_error_then_the_end() {
    let mut stream = chunks(Broken);
    let err = stream.next().await.expect("an item").expect_err("an error");
    assert_eq!(err.kind(), ErrorKind::Network);
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn resizes_reach_the_channel_and_closing_closes_it() {
    let (tx, mut rx) = mpsc::channel::<KubeSize>(4);
    let mut sink = resize_sink(tx);
    sink.send(TerminalSize::new(120, 40)).await.expect("resize");
    sink.send(TerminalSize::new(80, 24)).await.expect("resize");
    sink.close().await.expect("close");
    let first = rx.next().await.expect("first");
    assert_eq!((first.width, first.height), (120, 40));
    let second = rx.next().await.expect("second");
    assert_eq!((second.width, second.height), (80, 24));
    assert!(
        rx.next().await.is_none(),
        "closing the sink closes the channel"
    );
}

#[tokio::test]
async fn a_resize_after_the_session_ended_is_a_network_error() {
    let (tx, rx) = mpsc::channel::<KubeSize>(1);
    let mut sink = resize_sink(tx);
    drop(rx);
    let err = sink
        .send(TerminalSize::new(1, 1))
        .await
        .expect_err("nobody listens");
    assert_eq!(err.kind(), ErrorKind::Network);
}
