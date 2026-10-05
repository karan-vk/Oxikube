-- kv rows: `namespace = ''` is the free-form kv store (`kv_*`); any other namespace is a typed
-- table of `StatePort` (`table_*`), whose names are never empty.
CREATE TABLE kv (
    namespace  TEXT    NOT NULL,
    key        TEXT    NOT NULL,
    value      TEXT    NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (namespace, key)
) WITHOUT ROWID;

-- The audit log: append-only (ADR 0012). `record` is the AuditRecord as JSON (no bodies); the
-- other columns are what `AuditQuery` filters on.
CREATE TABLE audit_log (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ns   INTEGER NOT NULL,
    cluster TEXT    NOT NULL,
    cmd     TEXT    NOT NULL,
    record  TEXT    NOT NULL
);
CREATE INDEX audit_log_ts ON audit_log (ts_ns);
CREATE INDEX audit_log_cluster_ts ON audit_log (cluster, ts_ns);

CREATE TRIGGER audit_log_no_update BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'the audit log is append-only');
END;
CREATE TRIGGER audit_log_no_delete BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'the audit log is append-only');
END;
