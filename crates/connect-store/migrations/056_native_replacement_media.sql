-- Fresh-schema continuation custody. Predecessor databases remain backup-only.
CREATE TABLE native_replacement_media_steps (
 operation_record_id TEXT NOT NULL REFERENCES native_replacements(operation_record_id),
 phase TEXT NOT NULL CHECK(phase IN ('attach','activate','readiness')),
 attempt_id TEXT NOT NULL UNIQUE,
 effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id),
 started INTEGER NOT NULL DEFAULT 0 CHECK(started IN (0,1)),
 observation_json TEXT CHECK(observation_json IS NULL OR (started=1 AND json_valid(observation_json) AND length(observation_json)<=32768)),
 PRIMARY KEY(operation_record_id,phase)
);
CREATE TRIGGER native_replacement_media_binding BEFORE INSERT ON native_replacement_media_steps
 WHEN NOT EXISTS(SELECT 1 FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id
 JOIN operation_effects e ON e.operation_record_id=o.record_id AND e.id=NEW.effect_id
 WHERE n.operation_record_id=NEW.operation_record_id AND o.state='preparing'
 AND e.provider='native-replacement-'||NEW.phase AND e.state='prepared' AND e.attempt=0)
 BEGIN SELECT RAISE(ABORT,'native replacement media step mismatch'); END;
CREATE TRIGGER native_replacement_media_immutable BEFORE UPDATE ON native_replacement_media_steps
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.phase!=OLD.phase
 OR NEW.attempt_id!=OLD.attempt_id OR NEW.effect_id!=OLD.effect_id OR NEW.started<OLD.started
 OR (OLD.observation_json IS NOT NULL AND NEW.observation_json IS NOT OLD.observation_json)
 BEGIN SELECT RAISE(ABORT,'native replacement media step is immutable'); END;
CREATE TRIGGER native_replacement_media_retained BEFORE DELETE ON native_replacement_media_steps
 BEGIN SELECT RAISE(ABORT,'native replacement media step remains reserved'); END;
PRAGMA user_version=56;
