-- Frozen actual source and recipient incarnations; no queued permission readers.
CREATE TABLE native_dtmf_sources (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_stream_operations(operation_record_id),
 runtime_epoch TEXT NOT NULL,
 observation_json TEXT NOT NULL CHECK(json_valid(observation_json) AND json_type(observation_json)='object' AND length(CAST(observation_json AS BLOB))<=4096),
 sealed INTEGER NOT NULL DEFAULT 0 CHECK(sealed IN (0,1))
);
CREATE TRIGGER native_dtmf_source_binding BEFORE INSERT ON native_dtmf_sources
 WHEN NOT EXISTS(SELECT 1 FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id
 JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id
 JOIN connections c ON c.id=o.connection_id JOIN native_connection_lifetimes l ON l.operation_record_id=n.parent_record_id
 JOIN peer_signing_keys signing ON signing.key_id=n.key_id
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='dtmf.send' AND o.state='preparing'
 AND o.cancel_requested=0 AND n.closed=0 AND n.effect_started=1 AND e.state='submitting'
 AND n.attempt_runtime=NEW.runtime_epoch AND o.worker_runtime=n.attempt_runtime AND o.lease_owner=n.attempt_id
 AND l.runtime_epoch=n.attempt_runtime AND l.generation=n.generation AND l.retired=0
 AND c.generation=n.generation AND c.observed_runtime_id=n.attempt_runtime AND c.state IN ('ready','active')
 AND json_type(NEW.observation_json,'$.envelope_id')='text' AND length(json_extract(NEW.observation_json,'$.envelope_id')) BETWEEN 1 AND 128
 AND json_extract(NEW.observation_json,'$.stream_revision')=json_extract(c.readiness_json,'$.stream_revision')
 AND json_type(NEW.observation_json,'$.credential_revision')='integer' AND json_extract(NEW.observation_json,'$.credential_revision') BETWEEN 1 AND 9007199254740991
 AND json_extract(NEW.observation_json,'$.signing_key_id')=n.key_id
 AND json_extract(NEW.observation_json,'$.signing_algorithm')=signing.algorithm
 AND unixepoch(json_extract(NEW.observation_json,'$.received_at')) BETWEEN o.created_at AND o.expires_at
 AND NEW.sealed=0)
 BEGIN SELECT RAISE(ABORT,'native DTMF source binding mismatch'); END;
CREATE TRIGGER native_dtmf_source_immutable BEFORE UPDATE ON native_dtmf_sources
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.observation_json!=OLD.observation_json
 OR OLD.sealed!=0 OR NEW.sealed!=1 OR (SELECT COUNT(*) FROM native_dtmf_recipients r WHERE r.operation_record_id=OLD.operation_record_id) NOT BETWEEN 1 AND 512
 BEGIN SELECT RAISE(ABORT,'native DTMF source is immutable'); END;
CREATE TRIGGER native_dtmf_source_retained BEFORE DELETE ON native_dtmf_sources
 BEGIN SELECT RAISE(ABORT,'native DTMF source remains reserved'); END;
CREATE TABLE native_dtmf_recipients (
 operation_record_id TEXT NOT NULL REFERENCES native_dtmf_sources(operation_record_id),
 target_record_id TEXT NOT NULL REFERENCES native_connection_lifetimes(operation_record_id),
 registration_id TEXT NOT NULL CHECK(length(registration_id) BETWEEN 1 AND 128),
 target_json TEXT NOT NULL CHECK(json_valid(target_json) AND json_type(target_json)='object' AND length(CAST(target_json AS BLOB))<=4096),
 attempted INTEGER NOT NULL DEFAULT 0 CHECK(attempted IN (0,1)),
 frame_id TEXT,
 disposition TEXT CHECK(disposition IN ('not_started','local_submitted','outcome_unknown')),
 provenance TEXT CHECK(provenance IN ('local_writer','runtime_recovery')),
 observed_at INTEGER,
 journal_id TEXT UNIQUE,
 PRIMARY KEY(operation_record_id,target_record_id),
 UNIQUE(operation_record_id,registration_id),
 CHECK((attempted=0 AND frame_id IS NULL) OR (attempted=1 AND frame_id IS NOT NULL AND length(frame_id) BETWEEN 1 AND 128)),
 CHECK((disposition IS NULL AND provenance IS NULL AND observed_at IS NULL AND journal_id IS NULL) OR (disposition IS NOT NULL AND provenance IS NOT NULL AND observed_at IS NOT NULL AND journal_id IS NOT NULL)),
 CHECK(disposition NOT IN ('local_submitted','outcome_unknown') OR attempted=1),
 CHECK(provenance IS NOT 'runtime_recovery' OR disposition IN ('not_started','outcome_unknown'))
);
CREATE TRIGGER native_dtmf_recipient_binding BEFORE INSERT ON native_dtmf_recipients
 WHEN NOT EXISTS(SELECT 1 FROM native_dtmf_sources source JOIN operations o ON o.record_id=source.operation_record_id
 JOIN native_stream_operations n ON n.operation_record_id=o.record_id
 JOIN native_admission_sources target ON target.record_id=NEW.target_record_id
 JOIN native_admission_sources t ON t.operation_record_id=target.record_id
 JOIN native_connection_lifetimes l ON l.operation_record_id=target.record_id
 JOIN connections c ON c.id=target.connection_id
 WHERE source.operation_record_id=NEW.operation_record_id AND source.sealed=0 AND o.state='preparing' AND o.cancel_requested=0
 AND target.state='completed' AND target.conversation_id=o.conversation_id AND target.session_id=o.session_id
 AND target.record_id!=n.parent_record_id AND l.runtime_epoch=source.runtime_epoch AND l.retired=0
 AND t.attempt_runtime=l.runtime_epoch AND t.generation=l.generation AND c.generation=l.generation
 AND c.observed_runtime_id=l.runtime_epoch AND c.state IN ('ready','active')
 AND json_extract(NEW.target_json,'$.connection_id')=target.connection_id
 AND json_extract(NEW.target_json,'$.runtime_connection_id')=(SELECT runtime_connection_id FROM connection_offers WHERE connection_id=target.connection_id)
 AND json_extract(NEW.target_json,'$.participant_id')=t.participant_id AND json_extract(NEW.target_json,'$.membership_epoch')=t.membership_epoch
 AND json_extract(NEW.target_json,'$.generation')=t.generation AND json_extract(NEW.target_json,'$.key_id')=t.key_id
 AND json_extract(NEW.target_json,'$.device_id')=t.device_id AND json_extract(NEW.target_json,'$.endpoint_id')=t.endpoint_id
 AND json_extract(NEW.target_json,'$.identity_id')=target.owner_id
 AND NEW.attempted=0 AND NEW.frame_id IS NULL AND NEW.disposition IS NULL)
 BEGIN SELECT RAISE(ABORT,'native DTMF recipient binding mismatch'); END;
CREATE TRIGGER native_dtmf_recipient_immutable BEFORE UPDATE ON native_dtmf_recipients
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.target_record_id!=OLD.target_record_id OR NEW.registration_id!=OLD.registration_id OR NEW.target_json!=OLD.target_json
 OR OLD.disposition IS NOT NULL OR NEW.attempted<OLD.attempted
 OR (OLD.attempted=1 AND NEW.frame_id IS NOT OLD.frame_id)
 OR (NEW.disposition IS NULL AND NOT (OLD.attempted=0 AND NEW.attempted=1))
 BEGIN SELECT RAISE(ABORT,'native DTMF recipient outcome is immutable'); END;
CREATE TRIGGER native_dtmf_recipient_retained BEFORE DELETE ON native_dtmf_recipients
 BEGIN SELECT RAISE(ABORT,'native DTMF recipient remains reserved'); END;
CREATE INDEX native_dtmf_pending ON native_dtmf_recipients(operation_record_id) WHERE disposition IS NULL;
PRAGMA user_version=84;
