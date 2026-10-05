//! Terminal resizes as a `Sink`.

use futures::channel::mpsc;
use futures::{SinkExt, future};
use kube::api::TerminalSize as KubeSize;
use oxikube_domain::OxiError;
use oxikube_ports::TerminalSize;
use oxikube_ports::exec::ResizeSink;

/// A sink that forwards each [`TerminalSize`] to the websocket task, which sends it on the
/// resize channel. Closing the sink closes the channel; the task then stops waiting for
/// sizes and keeps serving the session.
pub(super) fn resize_sink(sender: mpsc::Sender<KubeSize>) -> ResizeSink {
    let sender = sender.sink_map_err(|_| OxiError::network("the exec session has ended"));
    Box::pin(sender.with(|size: TerminalSize| {
        future::ready(Ok::<_, OxiError>(KubeSize {
            width: size.width,
            height: size.height,
        }))
    }))
}
