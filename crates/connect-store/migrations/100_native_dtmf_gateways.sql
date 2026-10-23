-- A native source can freeze actual SIP recipients without inventing native
-- lifetimes. Both families share one sealed batch and one truthful aggregate.
CREATE TABLE native_dtmf_gateways (
 operation_record_id TEXT NOT NULL REFERENCES native_dtmf_sources(operation_record_id),
 target_record_id TEXT NOT NULL REFERENCES initial_telephone_operations(operation_record_id),
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
CREATE TRIGGER native_dtmf_gateway_binding BEFORE INSERT ON native_dtmf_gateways
 WHEN NOT EXISTS(SELECT 1 FROM native_dtmf_sources source
 JOIN operations o ON o.record_id=source.operation_record_id
 JOIN initial_telephone_operations t ON t.operation_record_id=NEW.target_record_id
 JOIN operations target ON target.record_id=t.operation_record_id
 JOIN initial_telephone_answers a ON a.operation_record_id=t.operation_record_id
 JOIN connections c ON c.id=target.connection_id
 WHERE source.operation_record_id=NEW.operation_record_id AND source.sealed=0
 AND o.state='preparing' AND o.cancel_requested=0 AND target.state='completed'
 AND target.conversation_id=o.conversation_id AND target.session_id=o.session_id
 AND a.accepted=1 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL
 AND t.attempt_runtime=source.runtime_epoch AND t.signaling_started=1
 AND c.generation=1 AND c.observed_runtime_id=source.runtime_epoch AND c.state IN ('ready','active')
 AND json_extract(NEW.target_json,'$.connection_id')=c.id
 AND json_extract(NEW.target_json,'$.runtime_connection_id')=t.runtime_connection_id
 AND json_extract(NEW.target_json,'$.sip_call_id')=t.sip_call_id
 AND json_extract(NEW.target_json,'$.participant_id')=t.target_participant_id
 AND json_extract(NEW.target_json,'$.membership_epoch')=c.membership_epoch
 AND json_extract(NEW.target_json,'$.generation')=c.generation
 AND json_extract(NEW.target_json,'$.identity_id')=c.identity_id
 AND json_extract(NEW.target_json,'$.endpoint_id')=t.endpoint_id
 AND json_extract(NEW.target_json,'$.endpoint_revision')=json_extract(t.source_json,'$.endpoint_revision')
 AND json_extract(NEW.target_json,'$.device_id')=json_extract(t.source_json,'$.device_id')
 AND json_extract(NEW.target_json,'$.endpoint_credential_id')=json_extract(t.policy_json,'$.endpoint_credential_id')
 AND json_extract(NEW.target_json,'$.route_credential_id')=json_extract(t.policy_json,'$.route_credential_id')
 AND json_extract(NEW.target_json,'$.route_owner_id')=json_extract(t.source_json,'$.route_owner_id')
 AND NEW.attempted=0 AND NEW.frame_id IS NULL AND NEW.disposition IS NULL)
 BEGIN SELECT RAISE(ABORT,'native DTMF SIP recipient binding mismatch'); END;
CREATE TRIGGER native_dtmf_gateway_immutable BEFORE UPDATE ON native_dtmf_gateways
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.target_record_id!=OLD.target_record_id OR NEW.registration_id!=OLD.registration_id OR NEW.target_json!=OLD.target_json
 OR OLD.disposition IS NOT NULL OR NEW.attempted<OLD.attempted
 OR (OLD.attempted=1 AND NEW.frame_id IS NOT OLD.frame_id)
 OR (NEW.disposition IS NULL AND NOT (OLD.attempted=0 AND NEW.attempted=1))
 BEGIN SELECT RAISE(ABORT,'native DTMF SIP recipient outcome is immutable'); END;
CREATE TRIGGER native_dtmf_gateway_retained BEFORE DELETE ON native_dtmf_gateways
 BEGIN SELECT RAISE(ABORT,'native DTMF SIP recipient remains reserved'); END;
CREATE INDEX native_dtmf_gateway_pending ON native_dtmf_gateways(operation_record_id) WHERE disposition IS NULL;
CREATE VIEW native_dtmf_deliveries AS
 SELECT operation_record_id,target_record_id,registration_id,target_json,attempted,frame_id,disposition,'native' AS target_kind FROM native_dtmf_recipients
 UNION ALL
 SELECT operation_record_id,target_record_id,registration_id,target_json,attempted,frame_id,disposition,'sip-initial' AS target_kind FROM native_dtmf_gateways;
DROP TRIGGER native_dtmf_source_immutable;
CREATE TRIGGER native_dtmf_source_immutable BEFORE UPDATE ON native_dtmf_sources
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.observation_json!=OLD.observation_json
 OR OLD.sealed!=0 OR NEW.sealed!=1
 OR (SELECT COUNT(*) FROM native_dtmf_deliveries r WHERE r.operation_record_id=OLD.operation_record_id) NOT BETWEEN 1 AND 512
 OR EXISTS(SELECT 1 FROM native_dtmf_deliveries r WHERE r.operation_record_id=OLD.operation_record_id GROUP BY registration_id HAVING COUNT(*)!=1)
 BEGIN SELECT RAISE(ABORT,'native DTMF source is immutable'); END;
PRAGMA user_version=100;
