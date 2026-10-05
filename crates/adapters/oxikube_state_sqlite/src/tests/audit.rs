//! The append-only audit log.

use futures::executor::block_on;
use oxikube_domain::audit::AuditOutcome::{Denied, Failed, Succeeded};
use oxikube_ports::{AuditQuery, StatePort};

use super::{Fixture, cluster, record, ts};

#[test]
fn append_and_query_newest_first_with_filters() {
    let fx = Fixture::new();
    let state = fx.open();
    block_on(async {
        state
            .append_audit(&[
                record("prod", "pod::Delete", "2026-10-01T10:00:00Z", Succeeded),
                record("prod", "workload::Scale", "2026-10-01T11:00:00Z", Failed),
                record("dev", "pod::Delete", "2026-10-01T12:00:00Z", Denied),
            ])
            .await
            .unwrap();
        state
            .append_audit(&[record(
                "prod",
                "pod::Delete",
                "2026-10-01T13:00:00Z",
                Succeeded,
            )])
            .await
            .unwrap();

        let all = state.query_audit(&AuditQuery::default()).await.unwrap();
        let times: Vec<_> = all.iter().map(|r| r.ts.to_string()).collect();
        assert_eq!(
            times,
            [
                "2026-10-01T13:00:00Z",
                "2026-10-01T12:00:00Z",
                "2026-10-01T11:00:00Z",
                "2026-10-01T10:00:00Z"
            ]
        );
        assert_eq!(
            all[1],
            record("dev", "pod::Delete", "2026-10-01T12:00:00Z", Denied)
        );

        let prod = state
            .query_audit(&AuditQuery {
                cluster: Some(cluster("prod")),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(prod.len(), 3);

        let deletes = state
            .query_audit(&AuditQuery {
                cmd: Some("pod::Delete".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(deletes.len(), 3);

        // `since` is inclusive, `until` exclusive.
        let window = state
            .query_audit(&AuditQuery {
                since: Some(ts("2026-10-01T11:00:00Z")),
                until: Some(ts("2026-10-01T13:00:00Z")),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(window.len(), 2);

        let limited = state
            .query_audit(&AuditQuery {
                limit: 1,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].ts, ts("2026-10-01T13:00:00Z"));
    });
}

#[test]
fn equal_timestamps_come_back_last_appended_first() {
    let fx = Fixture::new();
    let state = fx.open();
    block_on(async {
        let a = record("prod", "a::First", "2026-10-01T10:00:00Z", Succeeded);
        let b = record("prod", "b::Second", "2026-10-01T10:00:00Z", Succeeded);
        state.append_audit(&[a.clone(), b.clone()]).await.unwrap();
        assert_eq!(
            state.query_audit(&AuditQuery::default()).await.unwrap(),
            [b, a]
        );
    });
}

#[test]
fn the_log_survives_a_reopen_and_empty_appends_are_fine() {
    let fx = Fixture::new();
    block_on(async {
        let state = fx.open_async().await;
        state.append_audit(&[]).await.unwrap();
        state
            .append_audit(&[record(
                "prod",
                "pod::Delete",
                "2026-10-01T10:00:00Z",
                Succeeded,
            )])
            .await
            .unwrap();
        drop(state);
        let state = fx.open_async().await;
        assert_eq!(
            state
                .query_audit(&AuditQuery::default())
                .await
                .unwrap()
                .len(),
            1
        );
    });
}

#[test]
fn the_log_is_append_only_in_the_database_itself() {
    let fx = Fixture::new();
    block_on(async {
        let state = fx.open_async().await;
        state
            .append_audit(&[record(
                "prod",
                "pod::Delete",
                "2026-10-01T10:00:00Z",
                Succeeded,
            )])
            .await
            .unwrap();
        drop(state);
    });
    let conn = rusqlite::Connection::open(fx.path()).unwrap();
    let update = conn.execute("UPDATE audit_log SET cmd = 'x'", []);
    let delete = conn.execute("DELETE FROM audit_log", []);
    assert!(update.unwrap_err().to_string().contains("append-only"));
    assert!(delete.unwrap_err().to_string().contains("append-only"));
}
