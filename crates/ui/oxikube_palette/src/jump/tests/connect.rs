//! A jump to a context that is not connected: connect first, the rest once the session is up.

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;

use super::{Fixture, id, wait_for_data};
use crate::jump::CONNECT_WAIT;

fn jump_to_staging(f: &mut Fixture) {
    f.open();
    wait_for_data(f);
    f.type_text("pods @staging");
    f.keys("enter");
}

#[gpui::test]
fn the_list_opens_once_the_context_is_connected(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    jump_to_staging(&mut f);
    assert_eq!(
        f.take_sent(),
        [Command::ClusterConnect {
            cluster: id("staging")
        }],
        "the connect goes first, alone"
    );

    f.connector
        .ports_for(&id("staging"))
        .discovery
        .set_kinds(oxikube_testkit::kinds::core_kinds());
    block_on(f.sessions.connect(&id("staging"))).expect("connect staging");
    f.settle();
    assert_eq!(
        f.take_sent(),
        [
            Command::ClusterSelect {
                cluster: id("staging")
            },
            Command::ResourceOpenList {
                cluster: id("staging"),
                gvk: Gvk::new("", "v1", "Pod"),
            },
        ]
    );
}

#[gpui::test]
fn a_context_that_never_connects_ends_the_wait_without_the_rest(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    jump_to_staging(&mut f);
    let _ = f.take_sent();
    f.vcx
        .executor()
        .advance_clock(CONNECT_WAIT + std::time::Duration::from_secs(1));
    f.settle();
    assert_eq!(
        f.take_sent(),
        [],
        "nothing is opened in a cluster that is not there"
    );
}

const NOT_CONNECTED: &str = "The cluster did not connect, so the jump was not made.";

#[gpui::test]
fn a_connect_that_succeeds_shows_no_toast(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    jump_to_staging(&mut f);
    f.connector
        .ports_for(&id("staging"))
        .discovery
        .set_kinds(oxikube_testkit::kinds::core_kinds());
    block_on(f.sessions.connect(&id("staging"))).expect("connect staging");
    f.settle();
    assert_eq!(f.toasts(), Vec::<String>::new());
}

#[gpui::test]
fn a_timeout_says_the_jump_was_not_made(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    jump_to_staging(&mut f);
    let _ = f.take_sent();
    assert_eq!(f.toasts(), Vec::<String>::new(), "nothing while it waits");
    f.vcx
        .executor()
        .advance_clock(CONNECT_WAIT + std::time::Duration::from_secs(1));
    f.settle();
    assert_eq!(f.toasts(), [NOT_CONNECTED]);
}

#[gpui::test]
fn a_connect_that_needs_credentials_says_so_at_once(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    jump_to_staging(&mut f);
    let _ = f.take_sent();
    f.connector
        .connect_script_for(&id("staging"))
        .push_err(OxiError::auth("token expired", false));
    // The session ends in AuthRequired: waiting longer would not help.
    block_on(f.sessions.connect(&id("staging"))).expect("the attempt ends in a state");
    f.settle();
    assert_eq!(f.toasts(), [NOT_CONNECTED], "no wait for the timeout");
    assert_eq!(f.take_sent(), [], "and the rest of the jump is not sent");
}

#[gpui::test]
fn a_newer_jump_replaces_the_wait_of_an_older_one(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    jump_to_staging(&mut f);
    let _ = f.take_sent();
    // Jump somewhere else while staging is still connecting.
    f.run_line("pods");
    let _ = f.take_sent();
    f.connector
        .ports_for(&id("staging"))
        .discovery
        .set_kinds(oxikube_testkit::kinds::core_kinds());
    block_on(f.sessions.connect(&id("staging"))).expect("connect staging");
    f.settle();
    assert_eq!(
        f.take_sent(),
        [],
        "the older jump was dropped with its wait"
    );
}

/// A session that an earlier connect left in `AuthRequired` (credentials expired).
fn leave_staging_needing_credentials(f: &mut Fixture) {
    f.connector
        .connect_script_for(&id("staging"))
        .push_err(OxiError::auth("token expired", false));
    block_on(f.sessions.connect(&id("staging"))).expect("the first attempt ends in a state");
}

#[gpui::test]
fn an_old_failed_session_does_not_end_the_wait_before_the_connect_starts(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    leave_staging_needing_credentials(&mut f);
    jump_to_staging(&mut f);
    f.settle();
    assert_eq!(
        f.toasts(),
        Vec::<String>::new(),
        "the old failure is not this attempt's"
    );

    // The retry works this time.
    f.connector
        .ports_for(&id("staging"))
        .discovery
        .set_kinds(oxikube_testkit::kinds::core_kinds());
    block_on(f.sessions.connect(&id("staging"))).expect("retry staging");
    f.settle();
    assert_eq!(f.toasts(), Vec::<String>::new());
    assert_eq!(
        f.take_sent(),
        [
            Command::ClusterConnect {
                cluster: id("staging")
            },
            Command::ClusterSelect {
                cluster: id("staging")
            },
            Command::ResourceOpenList {
                cluster: id("staging"),
                gvk: Gvk::new("", "v1", "Pod"),
            },
        ]
    );
}

#[gpui::test]
fn a_retry_that_fails_again_still_says_so_at_once(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    leave_staging_needing_credentials(&mut f);
    jump_to_staging(&mut f);
    f.settle();
    let _ = f.take_sent();
    f.connector
        .connect_script_for(&id("staging"))
        .push_err(OxiError::auth("token expired", false));
    block_on(f.sessions.connect(&id("staging"))).expect("the retry ends in a state");
    f.settle();
    assert_eq!(f.toasts(), [NOT_CONNECTED], "no wait for the timeout");
    assert_eq!(f.take_sent(), []);
}
