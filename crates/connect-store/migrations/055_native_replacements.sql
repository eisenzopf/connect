-- Fresh schema only. Candidate IDs and native preparation custody are retained.
CREATE TABLE native_replacements (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 runtime_epoch TEXT NOT NULL,
 original_digest TEXT NOT NULL,
 reservation_id TEXT NOT NULL UNIQUE,
 candidate_connection_id TEXT NOT NULL UNIQUE,
 candidate_runtime_id TEXT NOT NULL UNIQUE,
 generation INTEGER NOT NULL CHECK(generation BETWEEN 2 AND 9007199254740991),
 plan_json TEXT NOT NULL CHECK(json_valid(plan_json) AND length(plan_json)<=32768),
 selection_json TEXT NOT NULL CHECK(json_valid(selection_json) AND length(selection_json)<=32768),
 plan_digest TEXT NOT NULL,
 expires_at INTEGER NOT NULL
);
CREATE TABLE native_replacement_steps (
 operation_record_id TEXT NOT NULL REFERENCES native_replacements(operation_record_id),
 phase TEXT NOT NULL CHECK(phase IN ('prepare','allocate','start','commit')),
 attempt_id TEXT NOT NULL UNIQUE,
 effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id),
 started INTEGER NOT NULL DEFAULT 0 CHECK(started IN (0,1)),
 observation_json TEXT CHECK(observation_json IS NULL OR (started=1 AND json_valid(observation_json) AND length(observation_json)<=32768)),
 PRIMARY KEY(operation_record_id,phase)
);
CREATE TRIGGER native_replacement_binding BEFORE INSERT ON native_replacements
 WHEN NOT EXISTS(SELECT 1 FROM operations o
 JOIN credentials k ON k.id=NEW.credential_id AND k.identity_id=o.owner_id AND k.revoked=0
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='connection.replace'
 AND o.state='preparing' AND o.digest=NEW.original_digest AND o.lease_owner=NEW.reservation_id
 AND o.lease_expires_at=NEW.expires_at)
 BEGIN SELECT RAISE(ABORT,'native replacement binding mismatch'); END;
CREATE TRIGGER native_replacement_immutable BEFORE UPDATE ON native_replacements
 BEGIN SELECT RAISE(ABORT,'native replacement reservation is immutable'); END;
CREATE TRIGGER native_replacement_retained BEFORE DELETE ON native_replacements
 BEGIN SELECT RAISE(ABORT,'native replacement ID remains reserved'); END;
CREATE TRIGGER native_replacement_step_binding BEFORE INSERT ON native_replacement_steps
 WHEN NOT EXISTS(SELECT 1 FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id
 JOIN operation_effects e ON e.operation_record_id=o.record_id AND e.id=NEW.effect_id
 WHERE n.operation_record_id=NEW.operation_record_id AND o.state='preparing'
 AND e.provider='native-replacement-'||NEW.phase AND e.state='prepared' AND e.attempt=0)
 BEGIN SELECT RAISE(ABORT,'native replacement step mismatch'); END;
CREATE TRIGGER native_replacement_step_immutable BEFORE UPDATE ON native_replacement_steps
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.phase!=OLD.phase
 OR NEW.attempt_id!=OLD.attempt_id OR NEW.effect_id!=OLD.effect_id OR NEW.started<OLD.started
 OR (OLD.observation_json IS NOT NULL AND NEW.observation_json IS NOT OLD.observation_json)
 BEGIN SELECT RAISE(ABORT,'native replacement step is immutable'); END;
CREATE TRIGGER native_replacement_step_retained BEFORE DELETE ON native_replacement_steps
 BEGIN SELECT RAISE(ABORT,'native replacement step remains reserved'); END;
PRAGMA user_version=55;
