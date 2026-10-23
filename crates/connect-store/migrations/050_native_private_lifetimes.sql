-- An established private context is independent of the execution lease. It
-- records actual decoded, confirmed application, not a future media IO permit.
CREATE TABLE native_private_lifetimes (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_consultation_attempts(operation_record_id),
 runtime_epoch TEXT NOT NULL,
 selection_json TEXT NOT NULL CHECK(json_valid(selection_json) AND length(selection_json)<=32768),
 audience_json TEXT NOT NULL CHECK(json_valid(audience_json) AND json_type(audience_json)='array'),
 established_at INTEGER NOT NULL,
 restored_at INTEGER
);
CREATE TRIGGER native_private_lifetime_binding BEFORE INSERT ON native_private_lifetimes
 WHEN NEW.restored_at IS NOT NULL OR NOT EXISTS(
 SELECT 1 FROM native_consultation_attempts n
 JOIN native_consultation_legs l ON l.operation_record_id=n.operation_record_id
 JOIN operations o ON o.record_id=n.operation_record_id
 JOIN operation_effects e ON e.id=n.effect_id
 WHERE n.operation_record_id=NEW.operation_record_id AND n.runtime_epoch=NEW.runtime_epoch
 AND n.selection_json=NEW.selection_json AND l.audience_json=NEW.audience_json
 AND n.started=1 AND n.applied_json IS NOT NULL AND n.decoded_audio_observed=1
 AND n.media_failure IS NULL AND e.state='confirmed'
 AND o.state IN ('committing','unknown'))
 BEGIN SELECT RAISE(ABORT,'private lifetime requires actual decoded application'); END;
CREATE TRIGGER native_private_lifetime_immutable BEFORE UPDATE ON native_private_lifetimes
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.runtime_epoch!=OLD.runtime_epoch
 OR NEW.selection_json!=OLD.selection_json OR NEW.audience_json!=OLD.audience_json
 OR NEW.established_at!=OLD.established_at
 OR (OLD.restored_at IS NOT NULL AND NEW.restored_at IS NOT OLD.restored_at)
 BEGIN SELECT RAISE(ABORT,'private lifetime origin is immutable'); END;
CREATE TRIGGER native_private_lifetime_retained BEFORE DELETE ON native_private_lifetimes
 BEGIN SELECT RAISE(ABORT,'private lifetime remains reserved'); END;
DROP VIEW native_private_content_floors;
CREATE VIEW native_private_content_floors AS
 SELECT connection_id,primary_connection_id AS partner_connection_id,
 operation_record_id,audience_json,1 AS is_candidate
 FROM native_consultation_legs
 UNION ALL
 SELECT l.primary_connection_id,l.connection_id,l.operation_record_id,l.audience_json,0
 FROM native_consultation_legs l
 JOIN native_consultation_attempts n ON n.operation_record_id=l.operation_record_id
 JOIN operations o ON o.record_id=l.operation_record_id
 JOIN operation_effects e ON e.id=n.effect_id
 WHERE o.state IN ('preparing','ready','committing','completed','unknown','interrupted')
 AND e.state IN ('prepared','submitting','confirmed','unknown')
 AND NOT EXISTS(SELECT 1 FROM native_private_lifetimes v
 WHERE v.operation_record_id=l.operation_record_id AND v.restored_at IS NOT NULL);
PRAGMA user_version=50;
