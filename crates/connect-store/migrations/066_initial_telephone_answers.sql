-- An actual captured answer is distinct from permission, media and live cleanup.
CREATE TABLE initial_telephone_answers (
 operation_record_id TEXT PRIMARY KEY REFERENCES initial_telephone_operations(operation_record_id),
 observation_json TEXT NOT NULL CHECK(json_valid(observation_json) AND length(observation_json)<=4096),
 codec TEXT NOT NULL CHECK(length(codec) BETWEEN 1 AND 64),
 observed_at INTEGER NOT NULL,
 accepted INTEGER NOT NULL DEFAULT 0 CHECK(accepted IN (0,1)),
 acceptance_record_id TEXT UNIQUE REFERENCES operations(record_id),
 connected_json TEXT CHECK(connected_json IS NULL OR (json_valid(connected_json) AND length(connected_json)<=65536)),
 connected_at INTEGER,
 retired_at INTEGER,
 CHECK((connected_json IS NULL)=(connected_at IS NULL)),
 CHECK(connected_at IS NULL OR accepted=1),
 CHECK(accepted=(acceptance_record_id IS NOT NULL))
);
CREATE TRIGGER initial_telephone_answer_binding BEFORE INSERT ON initial_telephone_answers
 WHEN NOT EXISTS(SELECT 1 FROM initial_telephone_operations n JOIN operation_effects e ON e.id=n.effect_id
 WHERE n.operation_record_id=NEW.operation_record_id AND n.signaling_started=1
 AND e.state IN ('submitting','unknown','confirmed')
 AND n.runtime_connection_id=json_extract(NEW.observation_json,'$.runtime_connection_id')
 AND n.sip_call_id=json_extract(NEW.observation_json,'$.call_id')
 AND NEW.codec=json_extract(NEW.observation_json,'$.codec'))
 BEGIN SELECT RAISE(ABORT,'initial answer has no matching submitted dialog'); END;
CREATE TRIGGER initial_telephone_answer_immutable BEFORE UPDATE ON initial_telephone_answers
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.observation_json!=OLD.observation_json
 OR NEW.codec!=OLD.codec OR NEW.observed_at!=OLD.observed_at OR NEW.accepted<OLD.accepted
 OR (OLD.acceptance_record_id IS NOT NULL AND NEW.acceptance_record_id IS NOT OLD.acceptance_record_id)
 OR (OLD.connected_json IS NOT NULL AND NEW.connected_json IS NOT OLD.connected_json)
 OR (OLD.connected_at IS NOT NULL AND NEW.connected_at IS NOT OLD.connected_at)
 OR (OLD.retired_at IS NOT NULL AND NEW.retired_at IS NOT OLD.retired_at)
 BEGIN SELECT RAISE(ABORT,'initial telephone observations are immutable'); END;
CREATE TRIGGER initial_telephone_acceptance_binding BEFORE UPDATE ON initial_telephone_answers
 WHEN NEW.accepted=1 AND NOT EXISTS(
 SELECT 1 FROM operations a JOIN operation_control_receipts r ON r.operation_record_id=a.record_id
 JOIN initial_telephone_operations n ON n.operation_record_id=NEW.operation_record_id
 JOIN operations p ON p.record_id=n.operation_record_id
 WHERE a.record_id=NEW.acceptance_record_id AND a.state='completed' AND a.command_type='session.accept'
 AND a.operation_id='answer_'||n.operation_record_id AND a.session_id=p.session_id AND a.conversation_id=p.conversation_id
 AND a.owner_id=json_extract(n.source_json,'$.target.identity_id')
 AND r.actor_participant_id=n.target_participant_id
 AND r.actor_membership_epoch=json_extract(n.source_json,'$.target.membership_epoch'))
 BEGIN SELECT RAISE(ABORT,'initial telephone acceptance has no matching own receipt'); END;
CREATE TRIGGER initial_telephone_answer_retained BEFORE DELETE ON initial_telephone_answers
 BEGIN SELECT RAISE(ABORT,'initial telephone answer identity remains reserved'); END;
CREATE TABLE initial_telephone_receive_projections (
 operation_record_id TEXT NOT NULL REFERENCES initial_telephone_answers(operation_record_id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL,
 before_revision INTEGER NOT NULL CHECK(before_revision BETWEEN 1 AND 9007199254740990),
 after_revision INTEGER NOT NULL CHECK(after_revision=before_revision+1),
 observation_json TEXT NOT NULL CHECK(json_valid(observation_json) AND length(observation_json)<=8192),
 recorded_at INTEGER NOT NULL,
 PRIMARY KEY(operation_record_id,participant_id),
 UNIQUE(operation_record_id,after_revision)
);
CREATE TRIGGER initial_receive_source_binding BEFORE INSERT ON initial_telephone_receive_projections
 WHEN NOT EXISTS(SELECT 1 FROM initial_telephone_operations n JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id JOIN json_each(n.source_json,'$.audience') m
 WHERE n.operation_record_id=NEW.operation_record_id AND a.connected_at IS NOT NULL AND a.retired_at IS NULL
 AND json_extract(m.value,'$.participant_id')=NEW.participant_id AND json_extract(m.value,'$.membership_epoch')=NEW.membership_epoch
 AND n.target_participant_id!=NEW.participant_id)
 BEGIN SELECT RAISE(ABORT,'initial receive projection source mismatch'); END;
CREATE TRIGGER initial_receive_projection_immutable BEFORE UPDATE ON initial_telephone_receive_projections BEGIN SELECT RAISE(ABORT,'initial receive observation is immutable'); END;
CREATE TRIGGER initial_receive_projection_retained BEFORE DELETE ON initial_telephone_receive_projections BEGIN SELECT RAISE(ABORT,'initial receive observation remains reserved'); END;
PRAGMA user_version=66;
