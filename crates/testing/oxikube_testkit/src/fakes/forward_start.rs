//! The `start` half of [`FakePortForwardPort`](super::FakePortForwardPort): a real loopback
//! listener that echoes every byte back, with scripted start failures and injectable
//! runtime errors, so the manager's restart logic can be tested without a cluster.

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use futures::channel::mpsc;
use oxikube_ports::{ForwardHandle, ForwardStats, PodRef, PortForwardError};
use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};

type ErrorSink = mpsc::UnboundedSender<PortForwardError>;

/// Scripted start failures and the live forwards' error sinks.
#[derive(Default)]
pub(super) struct ForwardStarts {
    failures: Mutex<VecDeque<PortForwardError>>,
    sinks: Mutex<Vec<ErrorSink>>,
}

impl std::fmt::Debug for ForwardStarts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardStarts")
            .field("queued_failures", &self.failures.lock().len())
            .finish_non_exhaustive()
    }
}

/// Stops the echo server when the handle drops.
struct Owner(JoinHandle<()>);

impl Drop for Owner {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl ForwardStarts {
    pub(super) fn fail_next(&self, err: PortForwardError) {
        self.failures.lock().push_back(err);
    }

    /// Reports `err` on every live forward. Returns how many received it.
    pub(super) fn inject(&self, err: &PortForwardError) -> usize {
        let mut sinks = self.sinks.lock();
        sinks.retain(|sink| !sink.is_closed());
        for sink in sinks.iter() {
            let _ = sink.unbounded_send(err.clone());
        }
        sinks.len()
    }

    pub(super) async fn start(
        &self,
        _target: PodRef,
        _remote_port: u16,
        bind: IpAddr,
        local_port: Option<u16>,
    ) -> Result<ForwardHandle, PortForwardError> {
        if let Some(err) = self.failures.lock().pop_front() {
            return Err(err);
        }
        let requested = SocketAddr::new(bind, local_port.unwrap_or(0));
        let listener =
            TcpListener::bind(requested)
                .await
                .map_err(|err| PortForwardError::BindFailed {
                    addr: requested,
                    kind: err.kind(),
                })?;
        let local_addr = listener
            .local_addr()
            .map_err(|err| PortForwardError::Other(err.to_string()))?;
        let stats = Arc::new(ForwardStats::default());
        let (tx, rx) = mpsc::unbounded();
        self.sinks.lock().push(tx);
        let task = tokio::spawn(echo_server(listener, stats.clone()));
        Ok(ForwardHandle::new(local_addr, stats, rx, Owner(task)))
    }
}

/// Accepts connections and echoes them; ends (with every connection) when aborted.
async fn echo_server(listener: TcpListener, stats: Arc<ForwardStats>) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    connections.spawn(echo(stream, stats.clone()));
                }
                Err(_) => break,
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
}

async fn echo(mut stream: TcpStream, stats: Arc<ForwardStats>) {
    let _active = stats.open_connection();
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                stats.add_bytes_out(n as u64);
                if stream.write_all(&buf[..n]).await.is_err() {
                    break;
                }
                stats.add_bytes_in(n as u64);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use futures::StreamExt;
    use oxikube_ports::PortForwardPort;

    use super::super::FakePortForwardPort;
    use super::*;

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    async fn echo_once(addr: SocketAddr, payload: &[u8]) -> Vec<u8> {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        stream.write_all(payload).await.expect("write");
        let mut back = vec![0; payload.len()];
        stream.read_exact(&mut back).await.expect("read");
        back
    }

    async fn eventually(mut done: impl FnMut() -> bool) {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition not reached");
    }

    #[tokio::test]
    async fn start_echoes_over_loopback_and_counts_traffic() {
        let fake = FakePortForwardPort::new();
        let handle = fake
            .start(PodRef::new("demo", "web"), 80, LOOPBACK, None)
            .await
            .expect("start");
        assert!(handle.local_addr.ip().is_loopback());
        assert_ne!(handle.local_addr.port(), 0, "None picks a free port");
        assert_eq!(echo_once(handle.local_addr, b"ping").await, b"ping");
        let stats = handle.stats.clone();
        eventually(|| {
            let s = stats.snapshot();
            s.bytes_in == 4
                && s.bytes_out == 4
                && s.total_connections == 1
                && s.active_connections == 0
        })
        .await;
    }

    #[tokio::test]
    async fn scripted_failures_come_first_then_starts_succeed() {
        let fake = FakePortForwardPort::new();
        fake.fail_next_start(PortForwardError::PodGone("demo/web".into()));
        let err = fake
            .start(PodRef::new("demo", "web"), 80, LOOPBACK, None)
            .await
            .unwrap_err();
        assert_eq!(err, PortForwardError::PodGone("demo/web".into()));
        assert!(
            fake.start(PodRef::new("demo", "web"), 80, LOOPBACK, None)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_busy_port_is_a_bind_failure() {
        let fake = FakePortForwardPort::new();
        let first = fake
            .start(PodRef::new("demo", "web"), 80, LOOPBACK, None)
            .await
            .expect("start");
        let err = fake
            .start(
                PodRef::new("demo", "web"),
                80,
                LOOPBACK,
                Some(first.local_addr.port()),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            PortForwardError::BindFailed {
                kind: std::io::ErrorKind::AddrInUse,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn injected_errors_reach_the_handle_and_stop_frees_the_port() {
        let fake = FakePortForwardPort::new();
        let mut handle = fake
            .start(PodRef::new("demo", "web"), 80, LOOPBACK, None)
            .await
            .expect("start");
        let addr = handle.local_addr;
        let err = PortForwardError::Disconnected("pod replaced".into());
        assert_eq!(fake.inject_error(&err), 1);
        assert_eq!(handle.errors.next().await, Some(err));
        handle.stop();
        eventually(|| std::net::TcpListener::bind(addr).is_ok()).await;
        assert_eq!(fake.inject_error(&PortForwardError::Other("x".into())), 0);
    }

    #[tokio::test]
    async fn start_calls_are_recorded() {
        let fake = FakePortForwardPort::new();
        let _h = fake
            .start(PodRef::new("demo", "web"), 8080, LOOPBACK, Some(0))
            .await
            .expect("start");
        assert_eq!(
            fake.recorded_calls(),
            vec![super::super::PortForwardCall::Start {
                target: PodRef::new("demo", "web"),
                remote_port: 8080,
                bind: LOOPBACK,
                local_port: Some(0),
            }]
        );
    }
}
