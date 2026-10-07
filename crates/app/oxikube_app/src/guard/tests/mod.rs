//! Guard tests against `oxikube_testkit` fakes through the [`CommandBus`](crate::CommandBus)
//! (see [`crate::testing::Harness`]).

mod audit;
mod confirm;
mod debug;
mod enforcement;
mod exec;
mod policy;
mod posture;
mod read_only;
