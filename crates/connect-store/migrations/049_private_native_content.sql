-- Private publication provenance is installed before the possible audience IO.
-- The fresh candidate is always private. The existing primary gains the same
-- floor while application is pending/applied/uncertain, until actual restoration.
ALTER TABLE native_consultation_attempts ADD COLUMN decoded_audio_observed INTEGER NOT NULL DEFAULT 0 CHECK(decoded_audio_observed IN (0,1));
ALTER TABLE native_consultation_attempts ADD COLUMN media_failure TEXT CHECK(media_failure IS NULL OR media_failure IN ('timeout','cancelled','primary-ended','remote-ended','consultant-ended','already-restored','native-confirmation-lost'));
CREATE TRIGGER native_consultation_media_observation BEFORE UPDATE ON native_consultation_attempts
 WHEN NEW.decoded_audio_observed<OLD.decoded_audio_observed
 OR (NEW.decoded_audio_observed=1 AND (NEW.started!=1 OR NEW.applied_json IS NULL))
 BEGIN SELECT RAISE(ABORT,'private decoded observation requires actual application custody'); END;
CREATE TRIGGER native_consultation_media_failure BEFORE UPDATE ON native_consultation_attempts
 WHEN (OLD.media_failure IS NOT NULL AND NEW.media_failure IS NOT OLD.media_failure)
 OR (NEW.media_failure IS NOT NULL AND (NEW.started!=1 OR NEW.applied_json IS NULL))
 BEGIN SELECT RAISE(ABORT,'private media failure requires actual application custody'); END;
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
 AND e.state IN ('prepared','submitting','confirmed','unknown');
PRAGMA user_version=49;
