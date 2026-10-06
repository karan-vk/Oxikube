//! Open, migration and the corrupt-state fallback.

use futures::executor::block_on;
use oxikube_domain::ErrorKind;
use oxikube_ports::StatePort;
use serde_json::json;

use super::{Fixture, key};
use crate::{SqliteState, migrations};

fn corrupt_files(fx: &Fixture) -> Vec<std::path::PathBuf> {
    let mut found: Vec<_> = std::fs::read_dir(fx.dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("state.db.corrupt-")
        })
        .collect();
    found.sort();
    found
}

#[test]
fn opening_creates_the_file_and_its_parent_directory_and_migrates() {
    let fx = Fixture::new();
    let nested = fx.dir.path().join("a/b/state.db");
    let state = block_on(SqliteState::open(&nested)).unwrap();
    assert!(nested.exists());
    assert!(state.recovery().is_none());
    assert_eq!(state.path(), nested);
    drop(state);
    let conn = rusqlite::Connection::open(&nested).unwrap();
    let version: u32 = conn
        .query_row("SELECT MAX(version) FROM migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, migrations::latest_version());
}

#[test]
fn an_empty_file_is_a_fresh_database_not_a_corrupt_one() {
    let fx = Fixture::new();
    std::fs::write(fx.path(), b"").unwrap();
    let state = fx.open();
    assert!(state.recovery().is_none());
    assert!(corrupt_files(&fx).is_empty());
}

#[test]
fn random_bytes_are_moved_aside_and_a_fresh_database_works() {
    let fx = Fixture::new();
    let junk: Vec<u8> = (0..8192u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    std::fs::write(fx.path(), &junk).unwrap();
    // Settings and keymap live next to the state in the config/data dirs; they must not be touched.
    let settings = fx.dir.path().join("settings.json");
    let keymap = fx.dir.path().join("keymap.json");
    std::fs::write(&settings, br#"{ "theme": "dark" }"#).unwrap();
    std::fs::write(&keymap, b"[]").unwrap();

    let state = fx.open();
    let recovery = state.recovery().expect("the corrupt file was recovered");
    assert_eq!(
        std::fs::read(&recovery.moved_to).unwrap(),
        junk,
        "kept for inspection"
    );
    assert_eq!(corrupt_files(&fx), std::slice::from_ref(&recovery.moved_to));

    block_on(async {
        assert_eq!(
            state.kv_get(&key("a")).await.unwrap(),
            None,
            "fresh and empty"
        );
        state.kv_set(&key("a"), json!(1)).await.unwrap();
        assert_eq!(state.kv_get(&key("a")).await.unwrap(), Some(json!(1)));
    });
    assert_eq!(std::fs::read(&settings).unwrap(), br#"{ "theme": "dark" }"#);
    assert_eq!(std::fs::read(&keymap).unwrap(), b"[]");

    // The next launch opens the fresh database normally.
    drop(state);
    let again = fx.open();
    assert!(again.recovery().is_none());
    assert_eq!(corrupt_files(&fx).len(), 1);
}

#[test]
fn a_damaged_real_database_is_recovered_with_its_sidecar_files() {
    let fx = Fixture::new();
    block_on(async {
        let state = fx.open_async().await;
        for i in 0..200 {
            state
                .kv_set(&key(&format!("k{i}")), json!("x".repeat(200)))
                .await
                .unwrap();
        }
    });
    // Smash the header of the real database and leave a stale WAL next to it.
    let mut bytes = std::fs::read(fx.path()).unwrap();
    bytes[..100].fill(0xAB);
    std::fs::write(fx.path(), &bytes).unwrap();
    let wal = fx.dir.path().join("state.db-wal");
    std::fs::write(&wal, b"stale wal").unwrap();

    let state = fx.open();
    let recovery = state.recovery().expect("recovered");
    // SQLite may delete a WAL it cannot use when it closes the damaged file; either way the stale
    // log must neither stay next to the fresh database nor be applied to it.
    let mut moved_wal = recovery.moved_to.clone().into_os_string();
    moved_wal.push("-wal");
    if let Ok(kept) = std::fs::read(moved_wal) {
        assert_eq!(kept, b"stale wal");
    }
    assert_ne!(std::fs::read(&wal).ok().as_deref(), Some(&b"stale wal"[..]));
    block_on(async {
        assert!(state.kv_list("").await.unwrap().is_empty());
    });
}

#[test]
fn a_second_corruption_does_not_overwrite_the_first_backup() {
    let fx = Fixture::new();
    std::fs::write(fx.path(), vec![7u8; 4096]).unwrap();
    drop(fx.open());
    std::fs::write(fx.path(), vec![9u8; 4096]).unwrap();
    drop(fx.open());
    let files = corrupt_files(&fx);
    assert_eq!(files.len(), 2);
    let contents: Vec<_> = files.iter().map(|f| std::fs::read(f).unwrap()[0]).collect();
    assert!(contents.contains(&7) && contents.contains(&9));
}

#[test]
fn a_database_from_a_newer_build_is_refused_and_left_alone() {
    let fx = Fixture::new();
    drop(fx.open());
    {
        let conn = rusqlite::Connection::open(fx.path()).unwrap();
        conn.execute(
            "INSERT INTO migrations (version, name, applied_at) VALUES (?1, 'future', 0)",
            [migrations::latest_version() + 1],
        )
        .unwrap();
    }
    let before = std::fs::read(fx.path()).unwrap();
    let err = block_on(SqliteState::open(fx.path())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    assert!(corrupt_files(&fx).is_empty(), "never moved aside");
    assert_eq!(std::fs::read(fx.path()).unwrap().len(), before.len());
}

#[test]
fn a_path_that_cannot_be_a_database_file_is_an_error_not_a_reset() {
    let fx = Fixture::new();
    // A directory where the file should be.
    std::fs::create_dir(fx.path()).unwrap();
    let err = block_on(SqliteState::open(fx.path())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Internal);
    assert!(corrupt_files(&fx).is_empty());
    assert!(fx.path().is_dir());
}
