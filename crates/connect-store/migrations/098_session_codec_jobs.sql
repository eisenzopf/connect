-- Policy acceptance and its frozen reconciliation work commit together. A
-- separate operation reports media effects; the ordinary receipt stays immutable.
ALTER TABLE session_control ADD COLUMN codec_policy_operation_id TEXT;
CREATE TABLE session_codec_batches (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 policy_operation_id TEXT NOT NULL,
 manager_credential_id TEXT NOT NULL REFERENCES credentials(id),
 policy_revision INTEGER NOT NULL CHECK(policy_revision>0),
 runtime_epoch TEXT NOT NULL,
 codec_preferences_json TEXT NOT NULL,
 expires_at INTEGER NOT NULL
);
CREATE TABLE session_codec_targets (
 batch_record_id TEXT NOT NULL REFERENCES session_codec_batches(operation_record_id),
 connection_id TEXT NOT NULL REFERENCES connections(id),
 generation INTEGER NOT NULL CHECK(generation>0),
 state TEXT NOT NULL CHECK(state IN ('queued','preparing','updated','unchanged','refused','unavailable','cancelled','superseded','unknown')),
 native_operation_record_id TEXT REFERENCES native_stream_operations(operation_record_id),
 PRIMARY KEY(batch_record_id,connection_id),
 UNIQUE(native_operation_record_id)
);
CREATE INDEX session_codec_pending ON session_codec_targets(state,batch_record_id);
CREATE TRIGGER session_codec_batch_immutable BEFORE UPDATE ON session_codec_batches BEGIN
 SELECT RAISE(ABORT,'codec batch provenance is immutable');
END;
CREATE TRIGGER session_codec_target_origin BEFORE UPDATE ON session_codec_targets
 WHEN NEW.batch_record_id!=OLD.batch_record_id OR NEW.connection_id!=OLD.connection_id OR NEW.generation!=OLD.generation
 OR (OLD.native_operation_record_id IS NOT NULL AND NEW.native_operation_record_id IS NOT OLD.native_operation_record_id)
 BEGIN SELECT RAISE(ABORT,'codec target provenance is immutable'); END;
CREATE TRIGGER session_codec_target_terminal BEFORE UPDATE ON session_codec_targets
 WHEN OLD.state IN ('updated','unchanged','refused','unavailable','cancelled','superseded') AND NEW.state!=OLD.state
 BEGIN SELECT RAISE(ABORT,'codec target outcome is immutable'); END;
PRAGMA user_version=98;
