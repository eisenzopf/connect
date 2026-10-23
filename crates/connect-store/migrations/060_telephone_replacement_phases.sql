-- Fresh-schema custody of actual private telephone attachment and confirmation.
CREATE TABLE telephone_replacement_phases (
 operation_record_id TEXT NOT NULL REFERENCES telephone_replacement_resources(operation_record_id),
 phase TEXT NOT NULL CHECK(phase IN ('attach','activate','readiness')),
 runtime_epoch TEXT NOT NULL,
 attempt_id TEXT NOT NULL UNIQUE,
 effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id),
 observation_json TEXT CHECK(observation_json IS NULL OR (json_valid(observation_json) AND length(observation_json)<=32768)),
 audience_json TEXT CHECK(audience_json IS NULL OR (phase='activate' AND json_valid(audience_json) AND length(audience_json)<=262144)),
 PRIMARY KEY(operation_record_id,phase)
);
CREATE TRIGGER telephone_replacement_phase_binding BEFORE INSERT ON telephone_replacement_phases
 WHEN NOT EXISTS(SELECT 1 FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id
 JOIN telephone_replacement_resources r ON r.operation_record_id=n.operation_record_id AND r.runtime_epoch=n.runtime_epoch AND r.observation_json IS NOT NULL
 JOIN operation_effects re ON re.id=r.effect_id AND re.state='confirmed'
 JOIN operation_effects e ON e.operation_record_id=o.record_id AND e.id=NEW.effect_id
 WHERE n.operation_record_id=NEW.operation_record_id AND n.runtime_epoch=NEW.runtime_epoch
 AND o.state='preparing' AND o.cancel_requested=0 AND e.provider='telephone-replacement-'||NEW.phase
 AND e.state='prepared' AND e.attempt=0)
 BEGIN SELECT RAISE(ABORT,'telephone replacement phase binding mismatch'); END;
CREATE TRIGGER telephone_replacement_phase_immutable BEFORE UPDATE ON telephone_replacement_phases
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.phase!=OLD.phase OR NEW.runtime_epoch!=OLD.runtime_epoch
 OR NEW.attempt_id!=OLD.attempt_id OR NEW.effect_id!=OLD.effect_id
 OR (OLD.observation_json IS NOT NULL AND NEW.observation_json IS NOT OLD.observation_json)
 OR (OLD.audience_json IS NOT NULL AND NEW.audience_json IS NOT OLD.audience_json)
 BEGIN SELECT RAISE(ABORT,'telephone replacement phase is immutable'); END;
CREATE TRIGGER telephone_replacement_phase_retained BEFORE DELETE ON telephone_replacement_phases
 BEGIN SELECT RAISE(ABORT,'telephone replacement phase remains reserved'); END;
PRAGMA user_version=60;
