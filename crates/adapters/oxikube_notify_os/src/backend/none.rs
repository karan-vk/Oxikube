//! Platforms without a backend (Windows, until it is added): always unavailable.

use super::{Backend, BackendError, Shown};
use crate::payload::Payload;

/// The backend of a platform this crate does not support yet.
pub(super) struct Unsupported;

impl Backend for Unsupported {
    fn connect(&self) -> Result<(), BackendError> {
        Err(BackendError::Unavailable(
            "this platform has no OS notification backend yet".to_owned(),
        ))
    }

    fn show(&self, _: &Payload, _: bool) -> Result<Box<dyn Shown>, BackendError> {
        self.connect()
            .map(|()| unreachable!("connect always fails"))
    }
}
