-- A pending row reserves one journal fact before a possible physical write.
-- Original source and target incarnations remain attributable after retirement.
CREATE TABLE native_data_writes (
 operation_record_id TEXT NOT NULL REFERENCES native_data_messages(operation_record_id),
 target_record_id TEXT NOT NULL REFERENCES native_connection_lifetimes(operation_record_id),
 runtime_epoch TEXT NOT NULL,
 disposition TEXT CHECK(disposition IN ('not_started','local_written','submission_unknown')),
 provenance TEXT CHECK(provenance IN ('local_writer','runtime_recovery')),
 observed_at INTEGER,
 journal_id TEXT UNIQUE,
 PRIMARY KEY(operation_record_id,target_record_id),
 CHECK((disposition IS NULL AND provenance IS NULL AND observed_at IS NULL AND journal_id IS NULL)
    OR (disposition IS NOT NULL AND provenance IS NOT NULL AND observed_at IS NOT NULL AND journal_id IS NOT NULL)),
 CHECK(provenance IS NOT 'runtime_recovery' OR disposition='submission_unknown')
);
CREATE TRIGGER native_data_write_binding BEFORE INSERT ON native_data_writes
 WHEN NOT EXISTS(SELECT 1 FROM native_stream_operations n
 JOIN operations o ON o.record_id=n.operation_record_id
 JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id
 JOIN operations target ON target.record_id=NEW.target_record_id
 JOIN native_offer_operations t ON t.operation_record_id=target.record_id
 JOIN native_data_recipients r ON r.operation_record_id=o.record_id
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='message.send'
 AND n.effect_started=1 AND n.attempt_runtime=NEW.runtime_epoch
 AND e.state IN ('submitting','confirmed','unknown')
 AND target.conversation_id=o.conversation_id AND target.session_id=o.session_id
 AND t.participant_id=r.participant_id AND t.membership_epoch=r.membership_epoch
 AND t.attempt_runtime=NEW.runtime_epoch AND t.effect_started=1
 AND NEW.disposition IS NULL)
 BEGIN SELECT RAISE(ABORT,'native data writer origin mismatch'); END;
CREATE TRIGGER native_data_write_immutable BEFORE UPDATE ON native_data_writes
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.target_record_id!=OLD.target_record_id
 OR NEW.runtime_epoch!=OLD.runtime_epoch OR OLD.disposition IS NOT NULL OR NEW.disposition IS NULL
 BEGIN SELECT RAISE(ABORT,'native data writer outcome is immutable'); END;
CREATE TRIGGER native_data_write_retained BEFORE DELETE ON native_data_writes
 BEGIN SELECT RAISE(ABORT,'native data writer identity remains reserved'); END;
CREATE INDEX native_data_write_pending ON native_data_writes(runtime_epoch) WHERE disposition IS NULL;
PRAGMA user_version=43;
