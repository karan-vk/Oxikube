//! Turning an HTTP status and body into a result: shared by the direct client and the service
//! proxy client, so both report a failure the same way.

use oxikube_ports::{PromqlError, PromqlSeries};

use crate::errors;
use crate::wire::{Reply, parse};

/// The start of a non-API body, for an error message.
fn snippet(body: &[u8]) -> String {
    String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned()
}

/// The series of a successful answer, or why it is not one.
pub(crate) fn decode(status: u16, body: &[u8]) -> Result<Vec<PromqlSeries>, PromqlError> {
    let success = (200..300).contains(&status);
    match parse(body) {
        Ok(Reply::Success(series)) if success => Ok(series),
        Ok(Reply::Success(_)) => Err(errors::from_server(status, "", "")),
        Ok(Reply::Failure {
            error_type,
            message,
        }) => {
            // An error envelope on a 2xx is still a failure; keep a server-side status.
            let status = if success { 500 } else { status };
            Err(errors::from_server(status, &error_type, &message))
        }
        // No envelope: judge by status first (a proxy's 401/502 page is not "bad response").
        Err(bad) if success => Err(bad),
        Err(_) => Err(errors::from_server(status, "", &snippet(body))),
    }
}
