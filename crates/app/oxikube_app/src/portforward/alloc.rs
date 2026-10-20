//! Automatic local-port allocation inside `portforward.port_range` (E15-S08).
//!
//! The OS cannot be told to pick from a range, so the supervisor walks the range from a
//! per-forward offset and takes the first port the listener can bind. Only "address in use"
//! moves on to the next candidate; every other error is the answer. The walk is bounded, so a
//! range that is full fails promptly with the last error (whose suggestion names the setting).

use std::net::IpAddr;

use oxikube_domain::portforward::PortRange;
use oxikube_ports::{ForwardHandle, PodRef, PortForwardError, PortForwardPort};

/// How many candidates one start tries before giving up.
pub(super) const MAX_TRIES: u32 = 64;

/// Starts the forward on the first bindable port of `range`, walking from `offset`.
pub(super) async fn start_in_range(
    port: &dyn PortForwardPort,
    pod: &PodRef,
    remote_port: u16,
    bind: IpAddr,
    range: PortRange,
    offset: u32,
) -> Result<ForwardHandle, PortForwardError> {
    let tries = range.len().min(MAX_TRIES);
    let mut last = None;
    for step in 0..tries {
        let candidate = range.nth_wrapping(offset.wrapping_add(step));
        match port
            .start(pod.clone(), remote_port, bind, Some(candidate))
            .await
        {
            Err(PortForwardError::BindFailed {
                kind: std::io::ErrorKind::AddrInUse,
                addr,
            }) => {
                last = Some(PortForwardError::BindFailed {
                    kind: std::io::ErrorKind::AddrInUse,
                    addr,
                });
            }
            other => return other,
        }
    }
    Err(last.unwrap_or_else(|| PortForwardError::Other(format!("no free port in {range}"))))
}
