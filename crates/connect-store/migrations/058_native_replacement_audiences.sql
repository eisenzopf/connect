-- Fresh-schema historical stream-revision custody for actual audience commits.
CREATE TABLE native_replacement_audiences (
 operation_record_id TEXT NOT NULL REFERENCES native_replacements(operation_record_id),
 phase TEXT NOT NULL CHECK(phase IN ('activate','commit')),
 effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id),
 runtime_epoch TEXT NOT NULL,
 capture_json TEXT NOT NULL CHECK(json_valid(capture_json) AND length(capture_json)<=262144),
 PRIMARY KEY(operation_record_id,phase)
);
CREATE TRIGGER native_replacement_audience_binding BEFORE INSERT ON native_replacement_audiences
 WHEN NOT EXISTS(SELECT 1 FROM native_replacements n WHERE n.operation_record_id=NEW.operation_record_id AND n.runtime_epoch=NEW.runtime_epoch
 AND ((NEW.phase='activate' AND EXISTS(SELECT 1 FROM native_replacement_media_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE s.operation_record_id=n.operation_record_id AND s.phase='activate' AND s.started=1 AND s.observation_json IS NOT NULL AND s.effect_id=NEW.effect_id))
 OR (NEW.phase='commit' AND EXISTS(SELECT 1 FROM native_replacement_steps s JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE s.operation_record_id=n.operation_record_id AND s.phase='commit' AND s.started=1 AND s.observation_json IS NOT NULL AND s.effect_id=NEW.effect_id))))
 BEGIN SELECT RAISE(ABORT,'replacement audience capture mismatch'); END;
CREATE TRIGGER native_replacement_audience_immutable BEFORE UPDATE ON native_replacement_audiences
 BEGIN SELECT RAISE(ABORT,'replacement audience capture is immutable'); END;
CREATE TRIGGER native_replacement_audience_retained BEFORE DELETE ON native_replacement_audiences
 BEGIN SELECT RAISE(ABORT,'replacement audience capture remains reserved'); END;
PRAGMA user_version=58;
