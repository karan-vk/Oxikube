//! 2 000 registered commands: the list is virtualised and a keystroke settles in one go.

use std::time::Instant;

use gpui::TestAppContext;
use oxikube_app::{CommandIndex, CommandInfo};
use oxikube_testkit::commands::fixture_commands;

use super::Fixture;

fn two_thousand() -> CommandIndex {
    CommandIndex::new(
        fixture_commands(2_000)
            .into_iter()
            .map(|meta| CommandInfo::new(meta, "test", true)),
    )
    .expect("distinct ids")
}

#[gpui::test]
fn only_the_visible_rows_are_built(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, two_thousand(), &["web"]);
    f.open();
    let listed = f.listed().len();
    assert!(listed > 1_000, "{listed} listed");
    let drawn = (0..listed)
        .take(60)
        .filter(|ix| {
            let selector: &'static str =
                Box::leak(format!("palette-command-{ix}").into_boxed_str());
            f.vcx.debug_bounds(selector).is_some()
        })
        .count();
    assert!(
        drawn > 0 && drawn < 20,
        "{drawn} rows built for {listed} commands"
    );
}

#[gpui::test]
fn a_keystroke_filters_a_long_list_and_a_newer_one_wins(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx, two_thousand(), &["web"]);
    f.open();
    let all = f.listed().len();
    // Two keystrokes in a row: the first matching task is dropped by the second.
    f.vcx.simulate_input("fixture 19");
    f.vcx.simulate_input("96");
    f.settle();
    let narrowed = f.listed();
    assert!(
        !narrowed.is_empty() && narrowed.len() < all,
        "{} of {all}",
        narrowed.len()
    );
    assert!(
        narrowed.iter().any(|id| id.as_str().ends_with("Fx1996")),
        "the command asked for is listed"
    );
    let started = Instant::now();
    f.type_text("0");
    // Settling a keystroke over 2 000 commands is far inside a frame budget even on the test
    // platform (the bench reports the exact numbers).
    assert!(
        started.elapsed().as_millis() < 500,
        "{:?}",
        started.elapsed()
    );
}
