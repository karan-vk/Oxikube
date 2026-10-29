//! Guard tests against `oxikube_testkit` fakes through the [`CommandBus`](crate::CommandBus)
//! (see [`crate::testing::Harness`]).

mod audit;
mod confirm;
mod debug;
mod dry_run;
mod enforcement;
mod exec;
mod policy;
mod posture;
mod provider;
mod read_only;
mod read_only_sweep;
