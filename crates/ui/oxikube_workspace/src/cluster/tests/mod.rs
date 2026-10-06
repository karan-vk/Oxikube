//! `#[gpui::test]`s of the cluster badges, the status item, the menu and the command runner,
//! over the real session manager, guard and command bus with testkit fakes.

mod badges;
mod fixture;
mod menu;
mod runner;

pub(super) use fixture::{Fixture, bounds, fixture};
