//! The audit store's columns, indices, keyset paging and retention (E19-S08).

use std::time::{Duration, Instant};

use futures::executor::block_on;
use oxikube_domain::audit::{AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::safety::MutationVerb;
use oxikube_ports::{AuditCursor, AuditQuery, StatePort};
use rusqlite::{Connection, params_from_iter};

use super::{Fixture, cluster, record, ts};
use crate::audit;

/// A record for `ctx` at second `n` of 2026-10-01 10:00 UTC.
fn at(ctx: &str, n: i64) -> AuditRecord {
    let stamp = ts("2026-10-01T10:00:00Z") + Duration::from_secs(n.unsigned_abs());
    record(
        ctx,
        "pod::Delete",
        &stamp.to_string(),
        AuditOutcome::Succeeded,
    )
}

async fn records(state: &impl StatePort, set: impl FnOnce(&mut AuditQuery)) -> Vec<AuditRecord> {
    let mut query = AuditQuery::default();
    set(&mut query);
    state.query_audit(&query).await.unwrap().records
}

async fn page_all(state: &impl StatePort, mut query: AuditQuery) -> Vec<Vec<AuditRecord>> {
    let mut pages = Vec::new();
    loop {
        let page = state.query_audit(&query).await.unwrap();
        pages.push(page.records);
        match page.next {
            Some(cursor) => query.cursor = Some(cursor),
            None => return pages,
        }
    }
}

#[test]
fn filters_by_verb_initiator_outcome_and_batch() {
    let fx = Fixture::new();
    let state = fx.open();
    let mut agent = at("prod", 1).with_verb(MutationVerb::Scale);
    agent.initiator = Initiator::Agent;
    let delete_in_batch = at("prod", 2)
        .with_verb(MutationVerb::Delete)
        .with_batch_id("b-1");
    let mut denied = at("prod", 3).with_verb(MutationVerb::Delete);
    denied.outcome = AuditOutcome::Denied;
    let old_style = at("dev", 4); // written without a verb
    block_on(async {
        state
            .append_audit(&[
                agent.clone(),
                delete_in_batch.clone(),
                denied.clone(),
                old_style,
            ])
            .await
            .unwrap();
        assert_eq!(
            records(&state, |x| x.verb = Some(MutationVerb::Scale)).await,
            [agent.clone()]
        );
        assert_eq!(
            records(&state, |x| x.verb = Some(MutationVerb::Delete)).await,
            [denied.clone(), delete_in_batch.clone()]
        );
        assert_eq!(
            records(&state, |x| x.initiator = Some(Initiator::Agent)).await,
            [agent]
        );
        assert_eq!(
            records(&state, |x| x.outcome = Some(AuditOutcome::Denied)).await,
            [denied.clone()]
        );
        assert_eq!(
            records(&state, |x| x.batch_id = Some("b-1".into())).await,
            std::slice::from_ref(&delete_in_batch)
        );
        // Filters combine.
        assert_eq!(
            records(&state, |x| {
                x.verb = Some(MutationVerb::Delete);
                x.outcome = Some(AuditOutcome::Succeeded);
                x.cluster = Some(cluster("prod"));
            })
            .await,
            [delete_in_batch]
        );
        // No verb filter matches a record written without one.
        assert_eq!(
            records(&state, |x| x.cluster = Some(cluster("dev")))
                .await
                .len(),
            1
        );
        assert!(
            records(&state, |x| {
                x.cluster = Some(cluster("dev"));
                x.verb = Some(MutationVerb::Delete);
            })
            .await
            .is_empty()
        );
    });
}

#[test]
fn keyset_paging_returns_every_row_once_even_with_equal_timestamps() {
    let fx = Fixture::new();
    let state = fx.open();
    // 25 records over 10 distinct seconds: many share a timestamp.
    let all: Vec<AuditRecord> = (0..25)
        .map(|i| {
            let mut r = at("prod", i / 3);
            r.who = format!("user-{i}").as_str().into();
            r
        })
        .collect();
    block_on(async {
        state.append_audit(&all).await.unwrap();
        let pages = page_all(
            &state,
            AuditQuery {
                limit: 7,
                ..Default::default()
            },
        )
        .await;
        assert_eq!(pages.iter().map(Vec::len).collect::<Vec<_>>(), [7, 7, 7, 4]);
        let mut seen: Vec<String> = pages.concat().iter().map(|r| r.who.to_string()).collect();
        // Newest first, ties last-appended first: the exact reverse of append order.
        let mut expected: Vec<String> = all.iter().rev().map(|r| r.who.to_string()).collect();
        assert_eq!(seen, expected);
        seen.sort();
        expected.sort();
        assert_eq!(seen, expected, "no row twice, none missing");

        // A page that ends exactly at the end has no next cursor.
        let exact = state
            .query_audit(&AuditQuery {
                limit: 25,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!((exact.records.len(), exact.next), (25, None));
    });
}

#[test]
fn a_cursor_stays_valid_while_new_records_arrive() {
    let fx = Fixture::new();
    let state = fx.open();
    block_on(async {
        state
            .append_audit(&(0..10).map(|i| at("prod", i)).collect::<Vec<_>>())
            .await
            .unwrap();
        let first = state
            .query_audit(&AuditQuery {
                limit: 4,
                ..Default::default()
            })
            .await
            .unwrap();
        state.append_audit(&[at("prod", 100)]).await.unwrap();
        let second = state
            .query_audit(&AuditQuery {
                limit: 100,
                cursor: first.next,
                ..Default::default()
            })
            .await
            .unwrap();
        // The newer record is not in the older pages; the rest continue seamlessly.
        assert_eq!(second.records.len(), 6);
        assert_eq!(second.records[0].ts, ts("2026-10-01T10:00:05Z"));
    });
}

/// The `EXPLAIN QUERY PLAN` lines of the SQL a query runs.
fn plan(conn: &Connection, query: &AuditQuery) -> String {
    let (clause, mut args) = audit::filter(query).unwrap();
    args.push(rusqlite::types::Value::Integer(10));
    let sql = format!("EXPLAIN QUERY PLAN {}", audit::select_sql(&clause));
    let mut stmt = conn.prepare(&sql).unwrap();
    let rows = stmt
        .query_map(params_from_iter(args), |r| r.get::<_, String>(3))
        .unwrap();
    rows.map(Result::unwrap).collect::<Vec<_>>().join(" | ")
}

#[test]
fn every_viewer_filter_is_answered_from_an_index_without_sorting() {
    let fx = Fixture::new();
    drop(fx.open());
    let conn = Connection::open(fx.path()).unwrap();
    let cases: [(&str, AuditQuery, &str); 6] = [
        ("newest", AuditQuery::default(), "audit_log_ts_id"),
        (
            "cluster and time",
            AuditQuery {
                cluster: Some(cluster("prod")),
                since: Some(ts("2026-10-01T10:00:00Z")),
                until: Some(ts("2026-10-02T10:00:00Z")),
                ..Default::default()
            },
            "audit_log_cluster_ts_id",
        ),
        (
            "initiator",
            AuditQuery {
                initiator: Some(Initiator::Agent),
                ..Default::default()
            },
            "audit_log_initiator_ts_id",
        ),
        (
            "verb",
            AuditQuery {
                verb: Some(MutationVerb::Delete),
                ..Default::default()
            },
            "audit_log_verb_ts_id",
        ),
        (
            "batch",
            AuditQuery {
                batch_id: Some("b".into()),
                ..Default::default()
            },
            "audit_log_batch",
        ),
        (
            "cursor",
            AuditQuery {
                cluster: Some(cluster("prod")),
                cursor: Some(AuditCursor::new(ts("2026-10-01T10:00:00Z"), 5)),
                ..Default::default()
            },
            "audit_log_cluster_ts_id",
        ),
    ];
    for (name, query, index) in cases {
        let plan = plan(&conn, &query);
        assert!(
            plan.contains(&format!("USING INDEX {index}")),
            "{name}: {plan}"
        );
        if name != "batch" {
            assert!(!plan.contains("TEMP B-TREE"), "{name} sorts: {plan}");
        }
    }
}

#[test]
fn a_thousand_records_append_in_one_transaction_well_under_a_second() {
    let fx = Fixture::new();
    let state = fx.open();
    let batch: Vec<AuditRecord> = (0..1000).map(|i| at("prod", i)).collect();
    let started = Instant::now();
    block_on(state.append_audit(&batch)).unwrap();
    let elapsed = started.elapsed();
    eprintln!("1000 audit records appended in {elapsed:?}");
    assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
    let page = block_on(state.query_audit(&AuditQuery {
        limit: 5000,
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(page.records.len(), 1000);
}

#[test]
fn the_first_page_of_ten_thousand_records_is_fast() {
    let fx = Fixture::new();
    let state = fx.open();
    let batch: Vec<AuditRecord> = (0..10_000)
        .map(|i| at(if i % 2 == 0 { "prod" } else { "dev" }, i))
        .collect();
    block_on(state.append_audit(&batch)).unwrap();
    let query = AuditQuery {
        cluster: Some(cluster("prod")),
        since: Some(ts("2026-10-01T10:00:00Z")),
        limit: 200,
        ..Default::default()
    };
    block_on(state.query_audit(&query)).unwrap(); // warm the statement cache
    let started = Instant::now();
    let page = block_on(state.query_audit(&query)).unwrap();
    let elapsed = started.elapsed();
    eprintln!("first page of 200 from 10000 records: {elapsed:?}");
    assert_eq!(page.records.len(), 200);
    assert!(elapsed < Duration::from_millis(100), "{elapsed:?}");
}

fn raw(fx: &Fixture) -> Connection {
    Connection::open(fx.path()).unwrap()
}

fn pragma(conn: &Connection, name: &str) -> i64 {
    conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn pruning_removes_only_records_before_the_cutoff_and_vacuums() {
    let fx = Fixture::new();
    let state = fx.open();
    // Fat enough that the file visibly shrinks.
    let batch: Vec<AuditRecord> = (0..3000)
        .map(|i| {
            let mut r = at("prod", i);
            r.who = "x".repeat(200).as_str().into();
            r
        })
        .collect();
    block_on(state.append_audit(&batch)).unwrap();
    let pages_before = pragma(&raw(&fx), "page_count");

    let removed =
        block_on(state.prune_audit(ts("2026-10-01T10:00:00Z") + Duration::from_secs(2000)))
            .unwrap();
    assert_eq!(removed, 2000);
    let left = block_on(state.query_audit(&AuditQuery {
        limit: 5000,
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(left.records.len(), 1000);
    assert_eq!(
        left.records.last().unwrap().ts,
        ts("2026-10-01T10:33:20Z"),
        "the cutoff itself is kept"
    );

    let conn = raw(&fx);
    assert_eq!(
        pragma(&conn, "auto_vacuum"),
        2,
        "a new database is incremental"
    );
    assert_eq!(
        pragma(&conn, "freelist_count"),
        0,
        "the freed pages were given back"
    );
    assert!(pragma(&conn, "page_count") < pages_before);

    // Nothing older remains: a second prune is a no-op.
    assert_eq!(
        block_on(state.prune_audit(ts("2026-10-01T10:00:00Z") + Duration::from_secs(2000)))
            .unwrap(),
        0
    );
}

#[test]
fn an_older_database_is_converted_to_incremental_vacuum_by_its_first_prune() {
    let fx = Fixture::new();
    {
        let mut conn = Connection::open(fx.path()).unwrap();
        conn.pragma_update(None, "auto_vacuum", 0).unwrap();
        crate::migrations::apply(&mut conn).unwrap();
        assert_eq!(pragma(&conn, "auto_vacuum"), 0);
    }
    let state = fx.open();
    block_on(state.append_audit(&(0..50).map(|i| at("prod", i)).collect::<Vec<_>>())).unwrap();
    assert_eq!(
        block_on(state.prune_audit(ts("2026-10-01T10:00:25Z"))).unwrap(),
        25
    );
    assert_eq!(pragma(&raw(&fx), "auto_vacuum"), 2);
}

#[test]
fn only_pruning_can_delete_and_nothing_can_update() {
    let fx = Fixture::new();
    let state = fx.open();
    block_on(state.append_audit(&[at("prod", 1), at("prod", 50)])).unwrap();
    drop(state);
    let conn = raw(&fx);
    let cutoff = |t: &str| ts(t).as_nanosecond() as i64;

    assert!(
        conn.execute("UPDATE audit_log SET cluster = 'x'", [])
            .is_err()
    );
    assert!(
        conn.execute("DELETE FROM audit_log", []).is_err(),
        "no cutoff parked"
    );

    // Even with a cutoff parked, a row at or after it cannot go.
    conn.execute(
        "INSERT INTO audit_prune (cutoff_ns) VALUES (?1)",
        [cutoff("2026-10-01T10:00:10Z")],
    )
    .unwrap();
    let err = conn.execute("DELETE FROM audit_log", []).unwrap_err();
    assert!(err.to_string().contains("append-only"), "{err}");
    assert_eq!(
        conn.execute(
            "DELETE FROM audit_log WHERE ts_ns < ?1",
            [cutoff("2026-10-01T10:00:10Z")]
        )
        .unwrap(),
        1
    );
    let left: i64 = conn
        .query_row("SELECT count(*) FROM audit_log", [], |r| r.get(0))
        .unwrap();
    assert_eq!(left, 1);
}
