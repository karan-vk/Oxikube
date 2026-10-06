//! The keyboard: arrows, Enter, `/`, and the keys of the commands.

use gpui::TestAppContext;
use oxikube_domain::command::Command;

use super::{Fixture, contexts, id};
use crate::catalog::FocusSearch;

#[gpui::test]
fn the_arrow_keys_move_the_selection_from_inside_the_search_field(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(5));
    assert_eq!(f.read(|v| v.model().selected_index()), Some(0));
    f.keys("down down");
    assert_eq!(f.read(|v| v.model().selected_index()), Some(2));
    f.keys("up");
    assert_eq!(f.read(|v| v.model().selected_index()), Some(1));
    f.keys("up up up");
    assert_eq!(
        f.read(|v| v.model().selected_index()),
        Some(0),
        "stops at the top"
    );
    f.keys("down down down down down down");
    assert_eq!(
        f.read(|v| v.model().selected_index()),
        Some(4),
        "stops at the bottom"
    );
}

#[gpui::test]
fn enter_in_the_search_field_connects_the_selected_cluster(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(5));
    f.keys("down down enter");
    assert_eq!(
        f.recorder.sent(),
        [Command::ClusterConnect {
            cluster: id("ctx-02")
        }]
    );
}

#[gpui::test]
fn slash_focuses_the_search_from_the_list_and_types_a_slash_in_the_field(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(5));
    // Move the focus to the catalog itself, off the field.
    f.focus_list();
    assert!(!f.read(|v| v.search_focused));

    f.keys("/");
    assert!(
        f.read(|v| v.search_focused),
        "`/` jumped to the search field"
    );
    assert_eq!(
        f.read(|v| v.model().query().to_owned()),
        "",
        "the key did not type"
    );

    f.type_text("a/b");
    assert_eq!(
        f.read(|v| v.model().query().to_owned()),
        "a/b",
        "inside the field `/` is text"
    );
}

#[gpui::test]
fn enter_connects_from_the_list_too(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(3));
    f.focus_list();
    f.keys("down enter");
    assert_eq!(
        f.recorder.sent(),
        [Command::ClusterConnect {
            cluster: id("ctx-01")
        }]
    );
}

#[gpui::test]
fn home_and_end_jump_in_the_list(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(6));
    f.focus_list();
    f.keys("end");
    assert_eq!(f.read(|v| v.model().selected_index()), Some(5));
    f.keys("home");
    assert_eq!(f.read(|v| v.model().selected_index()), Some(0));
}

#[gpui::test]
fn enter_with_nothing_listed_sends_nothing(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, vec![]);
    f.keys("enter");
    assert!(f.recorder.sent().is_empty());
}

#[gpui::test]
fn the_favourite_key_stars_the_selected_cluster_and_moves_it_to_the_top(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(4));
    let key = if cfg!(target_os = "macos") {
        "cmd-d"
    } else {
        "ctrl-d"
    };
    f.keys("down down");
    f.keys(key);
    assert_eq!(
        f.recorder.sent(),
        [Command::ClusterToggleFavourite {
            cluster: id("ctx-02"),
            favourite: Some(true)
        }]
    );
    assert_eq!(f.names()[0], "ctx-02", "favourites sort first");
    assert_eq!(
        f.read(|v| v.model().selected_index()),
        Some(0),
        "the selection moved with its row"
    );
    f.keys(key);
    assert_eq!(f.recorder.sent().len(), 2);
    assert_eq!(
        f.recorder.sent()[1],
        Command::ClusterToggleFavourite {
            cluster: id("ctx-02"),
            favourite: Some(false)
        }
    );
    assert_eq!(f.names()[0], "ctx-00");
}

#[gpui::test]
fn the_disconnect_key_sends_the_disconnect_command(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(3));
    let key = if cfg!(target_os = "macos") {
        "cmd-shift-d"
    } else {
        "ctrl-shift-d"
    };
    f.keys(&format!("down {key}"));
    assert_eq!(
        f.recorder.sent(),
        [Command::ClusterDisconnect {
            cluster: id("ctx-01")
        }]
    );
}

#[gpui::test]
fn the_focus_search_action_works_from_anywhere_in_the_catalog(cx: &mut TestAppContext) {
    let mut f = Fixture::open(cx, contexts(2));
    f.focus_list();
    f.window.dispatch_action(FocusSearch);
    f.app.run_until_parked();
    assert!(f.read(|v| v.search_focused));
}
