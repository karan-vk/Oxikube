-- The audit log's queryable columns, indices and retention (E19-S08).
--
-- `record` stays the one source of truth; the columns below are VIRTUAL generated columns over
-- it, so they cost no extra disk per row, cannot drift from the record, and need no backfill
-- (an index over a generated column is built from the rows already stored). The existing
-- append-only triggers still hold: nothing can update them.
ALTER TABLE audit_log ADD COLUMN verb      TEXT GENERATED ALWAYS AS (json_extract(record, '$.verb')) VIRTUAL;
ALTER TABLE audit_log ADD COLUMN initiator TEXT GENERATED ALWAYS AS (json_extract(record, '$.initiator')) VIRTUAL;
ALTER TABLE audit_log ADD COLUMN outcome   TEXT GENERATED ALWAYS AS (json_extract(record, '$.outcome')) VIRTUAL;
ALTER TABLE audit_log ADD COLUMN batch_id  TEXT GENERATED ALWAYS AS (json_extract(record, '$.batch_id')) VIRTUAL;
ALTER TABLE audit_log ADD COLUMN kind      TEXT GENERATED ALWAYS AS (json_extract(record, '$.target.gvk.kind')) VIRTUAL;
ALTER TABLE audit_log ADD COLUMN namespace TEXT GENERATED ALWAYS AS (json_extract(record, '$.target.namespace')) VIRTUAL;
ALTER TABLE audit_log ADD COLUMN name      TEXT GENERATED ALWAYS AS (json_extract(record, '$.target.name')) VIRTUAL;

-- Newest first, the order the viewer reads in; `id` breaks ties so a keyset cursor on
-- `(ts_ns, id)` is exact. These replace the ascending indices of 0001.
DROP INDEX audit_log_ts;
DROP INDEX audit_log_cluster_ts;
CREATE INDEX audit_log_ts_id ON audit_log (ts_ns DESC, id DESC);
CREATE INDEX audit_log_cluster_ts_id ON audit_log (cluster, ts_ns DESC, id DESC);
CREATE INDEX audit_log_initiator_ts_id ON audit_log (initiator, ts_ns DESC, id DESC);
CREATE INDEX audit_log_verb_ts_id ON audit_log (verb, ts_ns DESC, id DESC);
CREATE INDEX audit_log_batch ON audit_log (batch_id, id) WHERE batch_id IS NOT NULL;

-- Retention pruning is the only delete. `audit_prune` is empty except inside the pruning
-- transaction, which parks its cutoff here; the trigger refuses to delete any row at or after it
-- (and every row when no cutoff is parked). Defence in depth against a stray DELETE, not a
-- security boundary: whoever can write the file can change the trigger.
CREATE TABLE audit_prune (cutoff_ns INTEGER NOT NULL);
DROP TRIGGER audit_log_no_delete;
CREATE TRIGGER audit_log_prune_only BEFORE DELETE ON audit_log
WHEN OLD.ts_ns >= COALESCE((SELECT MIN(cutoff_ns) FROM audit_prune), -9223372036854775808)
BEGIN
    SELECT RAISE(ABORT, 'the audit log is append-only: only retention pruning removes records');
END;
