//! `StateRecents`: order, cap, persistence round trip, merge and the corrupt value.

use oxikube_domain::command::{self, CommandId};
use oxikube_ports::StateKey;
use serde_json::json;

use super::*;
use crate::command_bus::RecentsStore;
use crate::search::recents::{COMMAND_CAPACITY, RECENTS_KEY};

#[test]
fn the_latest_command_comes_first_without_repeats() {
    let state = fake();
    let recents = recents(&state);
    assert!(recents.recent().is_empty());
    recents.record(DELETE);
    recents.record(ZOOM);
    recents.record(DELETE);
    assert_eq!(recents.recent(), [DELETE, ZOOM]);
}

#[test]
fn recording_touches_memory_only() {
    let state = fake();
    let recents = recents(&state);
    recents.record(DELETE);
    recents.record(ZOOM);
    assert!(
        state.recorded_calls().is_empty(),
        "no I/O until the writer runs"
    );
    assert!(recents.is_dirty());
}

#[test]
fn the_list_is_capped_keeping_the_latest() {
    let state = fake();
    let recents = recents(&state);
    let all: Vec<_> = command::COMMANDS.iter().map(|meta| meta.id).collect();
    assert!(
        all.len() > COMMAND_CAPACITY + 5,
        "the registry has enough commands"
    );
    for id in &all {
        recents.record(*id);
    }
    let kept = recents.recent();
    assert_eq!(kept.len(), COMMAND_CAPACITY);
    assert_eq!(kept[0], *all.last().unwrap());
    assert_eq!(
        kept[COMMAND_CAPACITY - 1],
        all[all.len() - COMMAND_CAPACITY]
    );
}

#[test]
fn recents_survive_a_restart() {
    let state = fake();
    let first = recents(&state);
    first.record(DELETE);
    first.record(ZOOM);
    first.record(SHELL);
    block_on(first.flush());
    assert!(!first.is_dirty());

    // A new process: a new store over the same database.
    let second = recents(&state);
    assert!(second.recent().is_empty(), "nothing before load");
    block_on(second.load());
    assert_eq!(second.recent(), [SHELL, ZOOM, DELETE]);
    assert!(!second.is_dirty(), "loading is not a change");
}

#[test]
fn the_stored_value_holds_command_ids_and_nothing_else() {
    let state = fake();
    let recents = recents(&state);
    recents.record(DELETE);
    recents.record(ZOOM);
    block_on(recents.flush());
    assert_eq!(
        stored(&state, RECENTS_KEY),
        Some(json!({ "v": 1, "ids": ["view::ZoomIn", "pod::Delete"] }))
    );
    // One key, written once.
    assert_eq!(writes(&state).len(), 1);
}

#[test]
fn load_puts_the_stored_list_behind_what_ran_since_start() {
    let state = fake();
    let old = recents(&state);
    old.record(DELETE);
    old.record(ZOOM);
    block_on(old.flush());

    let fresh = recents(&state);
    fresh.record(SHELL);
    fresh.record(DELETE);
    block_on(fresh.load());
    // Newest first: what ran now, then what was stored, no repeats.
    assert_eq!(fresh.recent(), [DELETE, SHELL, ZOOM]);
    assert!(
        fresh.is_dirty(),
        "the records made before load still need writing"
    );
}

#[test]
fn ids_that_are_no_longer_commands_are_dropped() {
    let state = fake();
    block_on(state.kv_set(
        &StateKey::new(RECENTS_KEY).unwrap(),
        json!({ "v": 1, "ids": ["pod::Delete", "pod::Explode", 7, "nonsense", "view::ZoomIn"] }),
    ))
    .unwrap();
    let recents = recents(&state);
    block_on(recents.load());
    assert_eq!(recents.recent(), [DELETE, ZOOM]);
}

#[test]
fn a_corrupt_value_falls_back_to_empty_and_the_store_keeps_working() {
    for bad in [
        json!("not an object"),
        json!(42),
        json!(null),
        json!({ "ids": "pod::Delete" }),
        json!({ "v": 1 }),
        json!(["pod::Delete"]),
    ] {
        let state = fake();
        block_on(state.kv_set(&StateKey::new(RECENTS_KEY).unwrap(), bad.clone())).unwrap();
        let recents = recents(&state);
        block_on(recents.load());
        assert!(recents.recent().is_empty(), "{bad}");
        // The next command is remembered and replaces the garbage.
        recents.record(DELETE);
        block_on(recents.flush());
        assert_eq!(
            stored(&state, RECENTS_KEY),
            Some(json!({ "v": 1, "ids": ["pod::Delete"] })),
            "{bad}"
        );
    }
}

#[test]
fn an_empty_database_loads_nothing() {
    let state = fake();
    let recents = recents(&state);
    block_on(recents.load());
    assert!(recents.recent().is_empty());
    assert!(!recents.is_dirty());
}

#[test]
fn clearing_forgets_and_persists_the_empty_list() {
    let state = fake();
    let recents = recents(&state);
    recents.record(DELETE);
    block_on(recents.flush());
    recents.clear();
    assert!(recents.recent().is_empty());
    block_on(recents.flush());
    assert_eq!(
        stored(&state, RECENTS_KEY),
        Some(json!({ "v": 1, "ids": [] }))
    );
    let fresh = super::recents(&state);
    block_on(fresh.load());
    assert!(fresh.recent().is_empty(), "cleared recents stay cleared");
    // Clearing nothing changes nothing.
    fresh.clear();
    assert!(!fresh.is_dirty());
}

#[test]
fn a_flush_with_nothing_changed_makes_no_call() {
    let state = fake();
    let recents = recents(&state);
    block_on(recents.flush());
    recents.record(DELETE);
    block_on(recents.flush());
    state.clear_calls();
    block_on(recents.flush());
    recents.record(DELETE); // already first: not a change
    block_on(recents.flush());
    assert!(state.recorded_calls().is_empty());
}

#[test]
fn every_stored_id_is_a_registered_command() {
    // The round trip relies on `CommandId: FromStr` accepting exactly the registry.
    let id: CommandId = "pod::Delete".parse().unwrap();
    assert_eq!(id, DELETE);
    assert!("pod::Nope".parse::<CommandId>().is_err());
}
