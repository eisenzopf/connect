-- Physical SIP receive custody is separate from signed native ingress.
CREATE TABLE received_dtmf_sources (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 parent_record_id TEXT NOT NULL REFERENCES initial_telephone_operations(operation_record_id),
 runtime_epoch TEXT NOT NULL,
 observation_id TEXT NOT NULL CHECK(length(observation_id) BETWEEN 1 AND 128),
 source_kind TEXT NOT NULL CHECK(source_kind IN ('sip-info','rfc4733')),
 observation_json TEXT NOT NULL CHECK(json_valid(observation_json) AND length(CAST(observation_json AS BLOB))<=4096),
 source_json TEXT NOT NULL CHECK(json_valid(source_json) AND length(CAST(source_json AS BLOB))<=4096),
 sealed INTEGER NOT NULL DEFAULT 0 CHECK(sealed IN (0,1)),
 closed INTEGER NOT NULL DEFAULT 0 CHECK(closed IN (0,1)),
 UNIQUE(parent_record_id,source_kind,observation_id)
);
CREATE TABLE received_dtmf_recipients (
 operation_record_id TEXT NOT NULL REFERENCES received_dtmf_sources(operation_record_id),
 target_record_id TEXT NOT NULL REFERENCES native_connection_lifetimes(operation_record_id),
 registration_id TEXT NOT NULL CHECK(length(registration_id) BETWEEN 1 AND 128),
 target_json TEXT NOT NULL CHECK(json_valid(target_json) AND length(CAST(target_json AS BLOB))<=4096),
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
CREATE TRIGGER received_dtmf_source_binding BEFORE INSERT ON received_dtmf_sources
 WHEN NOT EXISTS(SELECT 1 FROM operations o JOIN initial_telephone_operations p ON p.operation_record_id=NEW.parent_record_id JOIN operations parent ON parent.record_id=p.operation_record_id JOIN initial_telephone_answers a ON a.operation_record_id=p.operation_record_id JOIN connections c ON c.id=parent.connection_id
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='dtmf.send' AND o.state='accepted'
 AND o.conversation_id=parent.conversation_id AND o.session_id=parent.session_id AND o.connection_id=c.id
 AND parent.state='completed' AND a.accepted=1 AND a.connected_at IS NOT NULL AND a.retired_at IS NULL
 AND c.state IN ('ready','active') AND c.generation=1 AND c.observed_runtime_id=NEW.runtime_epoch
 AND p.attempt_runtime=NEW.runtime_epoch AND p.signaling_started=1
 AND o.owner_id=c.identity_id AND json_extract(NEW.source_json,'$.connection_id')=c.id
 AND json_extract(NEW.source_json,'$.runtime_connection_id')=p.runtime_connection_id
 AND json_extract(NEW.source_json,'$.sip_call_id')=p.sip_call_id
 AND json_extract(NEW.source_json,'$.identity_id')=c.identity_id
 AND json_extract(NEW.source_json,'$.participant_id')=c.participant_id
 AND json_extract(NEW.source_json,'$.membership_epoch')=c.membership_epoch
 AND json_extract(NEW.source_json,'$.endpoint_id')=p.endpoint_id
 AND json_extract(NEW.source_json,'$.endpoint_revision')=json_extract(p.source_json,'$.endpoint_revision')
 AND json_extract(NEW.source_json,'$.device_id')=json_extract(p.source_json,'$.device_id')
 AND json_extract(NEW.source_json,'$.endpoint_credential_id')=json_extract(p.policy_json,'$.endpoint_credential_id')
 AND json_extract(NEW.source_json,'$.route_credential_id')=json_extract(p.policy_json,'$.route_credential_id')
 AND json_extract(NEW.source_json,'$.route_owner_id')=json_extract(p.source_json,'$.route_owner_id')
 AND json_extract(NEW.observation_json,'$.observation_id')=NEW.observation_id
 AND json_extract(NEW.observation_json,'$.kind')=NEW.source_kind AND NEW.sealed=0 AND NEW.closed=0)
 BEGIN SELECT RAISE(ABORT,'received DTMF source binding mismatch'); END;
CREATE TRIGGER received_dtmf_recipient_binding BEFORE INSERT ON received_dtmf_recipients
 WHEN NOT EXISTS(SELECT 1 FROM received_dtmf_sources source JOIN operations o ON o.record_id=source.operation_record_id JOIN native_admission_sources n ON n.operation_record_id=NEW.target_record_id JOIN operations target ON target.record_id=n.operation_record_id JOIN native_connection_lifetimes life ON life.operation_record_id=n.operation_record_id JOIN connections c ON c.id=target.connection_id JOIN connection_offers offer ON offer.connection_id=c.id
 WHERE source.operation_record_id=NEW.operation_record_id AND source.sealed=0 AND source.closed=0
 AND o.state='accepted' AND target.state='completed' AND life.retired=0
 AND target.conversation_id=o.conversation_id AND target.session_id=o.session_id
 AND c.generation=life.generation AND c.generation=n.generation AND c.state IN ('ready','active')
 AND c.observed_runtime_id=source.runtime_epoch AND life.runtime_epoch=source.runtime_epoch
 AND json_extract(NEW.target_json,'$.connection_id')=c.id
 AND json_extract(NEW.target_json,'$.runtime_connection_id')=offer.runtime_connection_id
 AND json_extract(NEW.target_json,'$.generation')=c.generation
 AND json_extract(NEW.target_json,'$.participant_id')=c.participant_id
 AND json_extract(NEW.target_json,'$.membership_epoch')=c.membership_epoch
 AND json_extract(NEW.target_json,'$.identity_id')=c.identity_id
 AND json_extract(NEW.target_json,'$.endpoint_id')=c.endpoint_id
 AND json_extract(NEW.target_json,'$.key_id')=n.key_id
 AND json_extract(NEW.target_json,'$.device_id')=n.device_id
 AND json_extract(NEW.target_json,'$.credential_id')=n.credential_id
 AND n.attempt_runtime=source.runtime_epoch
 AND NEW.attempted=0 AND NEW.frame_id IS NULL AND NEW.disposition IS NULL)
 BEGIN SELECT RAISE(ABORT,'received DTMF recipient binding mismatch'); END;
CREATE TRIGGER received_dtmf_source_immutable BEFORE UPDATE ON received_dtmf_sources
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.parent_record_id!=OLD.parent_record_id OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.observation_id!=OLD.observation_id OR NEW.source_kind!=OLD.source_kind OR NEW.observation_json!=OLD.observation_json OR NEW.source_json!=OLD.source_json
 OR NOT ((OLD.sealed=0 AND NEW.sealed=1 AND NEW.closed=0 AND (SELECT count(*) FROM received_dtmf_recipients r WHERE r.operation_record_id=OLD.operation_record_id)<=512)
 OR (OLD.sealed=1 AND NEW.sealed=1 AND OLD.closed=0 AND NEW.closed=1 AND NOT EXISTS(SELECT 1 FROM received_dtmf_recipients r WHERE r.operation_record_id=OLD.operation_record_id AND r.disposition IS NULL)))
 BEGIN SELECT RAISE(ABORT,'received DTMF source is immutable'); END;
CREATE TRIGGER received_dtmf_recipient_immutable BEFORE UPDATE ON received_dtmf_recipients
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.target_record_id!=OLD.target_record_id OR NEW.registration_id!=OLD.registration_id OR NEW.target_json!=OLD.target_json OR OLD.disposition IS NOT NULL OR NEW.attempted<OLD.attempted
 OR (OLD.attempted=1 AND NEW.frame_id IS NOT OLD.frame_id)
 OR (NEW.disposition IS NULL AND NOT(OLD.attempted=0 AND NEW.attempted=1))
 BEGIN SELECT RAISE(ABORT,'received DTMF recipient outcome is immutable'); END;
CREATE TRIGGER received_dtmf_source_retained BEFORE DELETE ON received_dtmf_sources BEGIN SELECT RAISE(ABORT,'received DTMF source remains reserved'); END;
CREATE TRIGGER received_dtmf_recipient_retained BEFORE DELETE ON received_dtmf_recipients BEGIN SELECT RAISE(ABORT,'received DTMF recipient remains reserved'); END;
CREATE INDEX received_dtmf_pending ON received_dtmf_sources(runtime_epoch) WHERE closed=0;
PRAGMA user_version=101;
