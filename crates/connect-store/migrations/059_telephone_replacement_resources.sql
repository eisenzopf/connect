-- Fresh-schema SIP resource custody; no native peer or ready flag is invented.
CREATE TABLE telephone_replacement_resources (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_replacements(operation_record_id),
 runtime_epoch TEXT NOT NULL,
 attempt_id TEXT NOT NULL UNIQUE,
 effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id),
 descriptor_json TEXT NOT NULL CHECK(json_valid(descriptor_json) AND length(descriptor_json)<=32768),
 observation_json TEXT CHECK(observation_json IS NULL OR (json_valid(observation_json) AND length(observation_json)<=32768))
);
CREATE TRIGGER telephone_replacement_resources_binding BEFORE INSERT ON telephone_replacement_resources
 WHEN NOT EXISTS(SELECT 1 FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id
 JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='start' AND s.started=1 AND s.observation_json IS NOT NULL
 JOIN operation_effects se ON se.id=s.effect_id AND se.state='confirmed'
 JOIN operation_effects e ON e.operation_record_id=o.record_id AND e.id=NEW.effect_id
 WHERE n.operation_record_id=NEW.operation_record_id AND n.runtime_epoch=NEW.runtime_epoch
 AND o.state='preparing' AND o.cancel_requested=0 AND e.provider='telephone-replacement-resources'
 AND e.state='prepared' AND e.attempt=0 AND e.observation_json=NEW.descriptor_json)
 BEGIN SELECT RAISE(ABORT,'telephone replacement resource binding mismatch'); END;
CREATE TRIGGER telephone_replacement_resources_immutable BEFORE UPDATE ON telephone_replacement_resources
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.runtime_epoch!=OLD.runtime_epoch
 OR NEW.attempt_id!=OLD.attempt_id OR NEW.effect_id!=OLD.effect_id OR NEW.descriptor_json!=OLD.descriptor_json
 OR (OLD.observation_json IS NOT NULL AND NEW.observation_json IS NOT OLD.observation_json)
 BEGIN SELECT RAISE(ABORT,'telephone replacement resources are immutable'); END;
CREATE TRIGGER telephone_replacement_resources_retained BEFORE DELETE ON telephone_replacement_resources
 BEGIN SELECT RAISE(ABORT,'telephone replacement resource attempt remains reserved'); END;
PRAGMA user_version=59;
