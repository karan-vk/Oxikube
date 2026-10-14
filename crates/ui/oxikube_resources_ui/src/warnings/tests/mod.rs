//! Tests of the warning badges and the Events tab over the app's events index: the real
//! `EventService` over `FakeEventsPort`, the real tables and drawer in the test window.

mod support;
mod tab;

pub(crate) use support::{EventsRig, warning_event};
