-- The committed replacement SIP incarnation has its own real lifetime. No
-- initial-call or native surrogate is inserted for an established gateway.
CREATE TABLE native_dtmf_established_gateways (
 operation_record_id TEXT NOT NULL REFERENCES native_dtmf_sources(operation_record_id),
 target_record_id TEXT NOT NULL REFERENCES telephone_connection_lifetimes(operation_record_id),
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
CREATE TRIGGER native_dtmf_established_gateway_binding BEFORE INSERT ON native_dtmf_established_gateways
 WHEN NOT EXISTS(SELECT 1 FROM native_dtmf_sources source
 JOIN operations o ON o.record_id=source.operation_record_id
 JOIN telephone_connection_lifetimes l ON l.operation_record_id=NEW.target_record_id
 JOIN telephone_replacement_commits t ON t.operation_record_id=l.operation_record_id
 JOIN operations target ON target.record_id=t.operation_record_id
 JOIN operation_effects e ON e.id=t.effect_id AND e.operation_record_id=t.operation_record_id
 JOIN native_replacements n ON n.operation_record_id=t.operation_record_id
 JOIN connections c ON c.id=l.connection_id
 WHERE source.operation_record_id=NEW.operation_record_id AND source.sealed=0
 AND o.state='preparing' AND o.cancel_requested=0 AND target.state='completed'
 AND target.conversation_id=o.conversation_id AND target.session_id=o.session_id
 AND l.retired=0 AND e.state='confirmed' AND t.observation_json IS NOT NULL AND t.audience_json IS NOT NULL
 AND l.runtime_epoch=source.runtime_epoch AND t.runtime_epoch=source.runtime_epoch AND n.runtime_epoch=source.runtime_epoch
 AND n.candidate_connection_id=c.id AND n.generation=l.generation
 AND c.generation=l.generation AND c.observed_runtime_id=source.runtime_epoch AND c.state IN ('ready','active')
 AND json_extract(NEW.target_json,'$.connection_id')=c.id
 AND json_extract(NEW.target_json,'$.runtime_connection_id')=n.candidate_runtime_id
 AND json_extract(NEW.target_json,'$.sip_call_id')=json_extract(t.descriptor_json,'$.media.call_id')
 AND json_extract(NEW.target_json,'$.participant_id')=c.participant_id
 AND json_extract(NEW.target_json,'$.membership_epoch')=c.membership_epoch
 AND json_extract(NEW.target_json,'$.generation')=c.generation
 AND json_extract(NEW.target_json,'$.identity_id')=c.identity_id
 AND json_extract(NEW.target_json,'$.endpoint_id')=c.endpoint_id
 AND json_extract(NEW.target_json,'$.device_id')=json_extract(t.descriptor_json,'$.device_id')
 AND json_extract(NEW.target_json,'$.route_credential_id')=json_extract(t.descriptor_json,'$.route_credential_id')
 AND json_extract(NEW.target_json,'$.route_owner_id')=json_extract(t.descriptor_json,'$.route_owner_id')
 AND NOT EXISTS(SELECT 1 FROM json_each(NEW.target_json) WHERE key IN ('endpoint_credential_id','signing_key_id','key_id','endpoint_revision'))
 AND NEW.attempted=0 AND NEW.frame_id IS NULL AND NEW.disposition IS NULL)
 BEGIN SELECT RAISE(ABORT,'native DTMF established SIP recipient binding mismatch'); END;
CREATE TRIGGER native_dtmf_established_gateway_immutable BEFORE UPDATE ON native_dtmf_established_gateways
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.target_record_id!=OLD.target_record_id OR NEW.registration_id!=OLD.registration_id OR NEW.target_json!=OLD.target_json
 OR OLD.disposition IS NOT NULL OR NEW.attempted<OLD.attempted
 OR (OLD.attempted=1 AND NEW.frame_id IS NOT OLD.frame_id)
 OR (NEW.disposition IS NULL AND NOT (OLD.attempted=0 AND NEW.attempted=1))
 BEGIN SELECT RAISE(ABORT,'native DTMF SIP recipient outcome is immutable'); END;
CREATE TRIGGER native_dtmf_established_gateway_retained BEFORE DELETE ON native_dtmf_established_gateways
 BEGIN SELECT RAISE(ABORT,'native DTMF SIP recipient remains reserved'); END;
CREATE INDEX native_dtmf_established_gateway_pending ON native_dtmf_established_gateways(operation_record_id) WHERE disposition IS NULL;
DROP VIEW native_dtmf_deliveries;
CREATE VIEW native_dtmf_deliveries AS
 SELECT operation_record_id,target_record_id,registration_id,target_json,attempted,frame_id,disposition,'native' AS target_kind FROM native_dtmf_recipients
 UNION ALL
 SELECT operation_record_id,target_record_id,registration_id,target_json,attempted,frame_id,disposition,'sip-initial' AS target_kind FROM native_dtmf_gateways
 UNION ALL
 SELECT operation_record_id,target_record_id,registration_id,target_json,attempted,frame_id,disposition,'sip-established' AS target_kind FROM native_dtmf_established_gateways;
PRAGMA user_version=102;
