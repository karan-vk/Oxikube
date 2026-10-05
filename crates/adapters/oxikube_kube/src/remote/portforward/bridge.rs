//! One accepted local connection bridged to one pod connection.
//!
//! Every connection opens its own `portforward` websocket (what kubectl does): a failure or a
//! slow client on one never touches another.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use oxikube_domain::OxiError;
use oxikube_ports::{PortForwardConnection, PortForwardPort};
use tokio::io::copy_bidirectional;
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_util::compat::FuturesAsyncReadCompatExt;

use super::hub::StatusHub;
use super::plan::Target;

/// How long, after the byte stream ended, to wait for the pod's error message. The server
/// sends the error before it closes the data channel, but the two reach us through different
/// tasks.
const ERROR_GRACE: Duration = Duration::from_millis(150);

/// What every bridge of one forward shares. Cheap to clone.
#[derive(Clone)]
pub(super) struct Bridge {
    pub(super) connector: Arc<dyn PortForwardPort>,
    pub(super) namespace: Arc<str>,
    pub(super) target: watch::Receiver<Option<Target>>,
    pub(super) hub: StatusHub,
    pub(super) local_addr: SocketAddr,
}

impl Bridge {
    /// Serves `local` until either side closes. Failures are published, never returned.
    pub(super) async fn serve(self, mut local: TcpStream) {
        let Some(target) = self.target.borrow().clone() else {
            // Between pods: refuse by closing, the client sees a reset.
            tracing::debug!("port-forward connection refused: no target pod right now");
            return;
        };
        // Interactive protocols (ssh, redis) must not wait for Nagle.
        let _ = local.set_nodelay(true);
        let connection = match self
            .connector
            .forward(&self.namespace, &target.pod, target.port)
            .await
        {
            Ok(connection) => connection,
            Err(err) => {
                tracing::debug!(kind = %err.kind(), "port-forward connection failed to open");
                self.hub.error(&err);
                return;
            }
        };
        self.hub.recover(self.local_addr, &target.pod);
        let PortForwardConnection { stream, closed } = connection;
        let mut remote = stream.compat();
        let mut closed: BoxFuture<'static, Option<OxiError>> = closed;

        let copy = copy_bidirectional(&mut local, &mut remote);
        tokio::pin!(copy);
        let mut closed_early = false;
        let result = loop {
            tokio::select! {
                result = &mut copy => break result,
                // A server-reported error ends the forward at once. A clean close (`None`)
                // does not: bytes may still be queued for the local side.
                reported = &mut closed, if !closed_early => {
                    closed_early = true;
                    if let Some(err) = reported {
                        self.hub.error(&err);
                        return;
                    }
                }
            }
        };
        if let Err(err) = &result {
            tracing::debug!(error = %err, "port-forward copy ended with an I/O error");
        }
        if !closed_early {
            // The stream ended first; the error that explains why may still be on its way.
            if let Ok(Some(err)) = tokio::time::timeout(ERROR_GRACE, closed).await {
                self.hub.error(&err);
            }
        }
    }
}
