-- New current-authority return intent; no repeated native submission.
CREATE TABLE native_consultation_returns (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 consultation_record_id TEXT NOT NULL REFERENCES native_consultation_attempts(operation_record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 runtime_epoch TEXT NOT NULL,
 original_digest TEXT NOT NULL,
 context_selection_json TEXT NOT NULL CHECK(json_valid(context_selection_json)),
 audience_json TEXT NOT NULL CHECK(json_valid(audience_json)),
 attempt_id TEXT UNIQUE,
 effect_id TEXT UNIQUE REFERENCES operation_effects(id),
 plan_json TEXT CHECK(plan_json IS NULL OR (json_valid(plan_json) AND length(plan_json)<=32768)),
 selection_json TEXT CHECK(selection_json IS NULL OR (json_valid(selection_json) AND length(selection_json)<=32768)),
 plan_digest TEXT,
 started INTEGER NOT NULL DEFAULT 0 CHECK(started IN (0,1)),
 applied_json TEXT CHECK(applied_json IS NULL OR (started=1 AND json_valid(applied_json) AND length(applied_json)<=32768))
);
CREATE TRIGGER native_return_binding BEFORE INSERT ON native_consultation_returns
 WHEN NOT EXISTS(SELECT 1 FROM operations o JOIN native_consultation_attempts n ON n.operation_record_id=NEW.consultation_record_id
 JOIN native_consultation_legs l ON l.operation_record_id=n.operation_record_id
 JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed'
 JOIN credentials k ON k.id=NEW.credential_id AND k.identity_id=o.owner_id AND k.revoked=0
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='session.consult.return' AND o.state='accepted' AND o.digest=NEW.original_digest
 AND n.started=1 AND n.applied_json IS NOT NULL AND n.runtime_epoch=NEW.runtime_epoch
 AND n.selection_json=NEW.context_selection_json AND l.audience_json=NEW.audience_json)
 BEGIN SELECT RAISE(ABORT,'native return context binding mismatch'); END;
CREATE TRIGGER native_return_immutable BEFORE UPDATE ON native_consultation_returns
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.consultation_record_id!=OLD.consultation_record_id
 OR NEW.credential_id!=OLD.credential_id OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.original_digest!=OLD.original_digest
 OR NEW.context_selection_json!=OLD.context_selection_json OR NEW.audience_json!=OLD.audience_json
 OR (OLD.attempt_id IS NOT NULL AND NEW.attempt_id IS NOT OLD.attempt_id)
 OR (OLD.effect_id IS NOT NULL AND NEW.effect_id IS NOT OLD.effect_id)
 OR (OLD.plan_json IS NOT NULL AND NEW.plan_json IS NOT OLD.plan_json)
 OR (OLD.selection_json IS NOT NULL AND NEW.selection_json IS NOT OLD.selection_json)
 OR (OLD.plan_digest IS NOT NULL AND NEW.plan_digest IS NOT OLD.plan_digest)
 OR NEW.started<OLD.started OR (OLD.applied_json IS NOT NULL AND NEW.applied_json IS NOT OLD.applied_json)
 BEGIN SELECT RAISE(ABORT,'native return is immutable'); END;
CREATE TRIGGER native_return_retained BEFORE DELETE ON native_consultation_returns
 BEGIN SELECT RAISE(ABORT,'native return remains reserved'); END;
CREATE UNIQUE INDEX one_uncertain_native_restore ON native_consultation_returns(consultation_record_id)
 WHERE started=1 AND applied_json IS NULL;
DROP VIEW native_private_content_floors;
CREATE VIEW native_private_content_floors AS
 SELECT connection_id,primary_connection_id AS partner_connection_id,operation_record_id,audience_json,1 AS is_candidate
 FROM native_consultation_legs
 UNION ALL
 SELECT l.primary_connection_id,l.connection_id,l.operation_record_id,l.audience_json,0
 FROM native_consultation_legs l JOIN native_consultation_attempts n ON n.operation_record_id=l.operation_record_id
 JOIN operations o ON o.record_id=l.operation_record_id JOIN operation_effects e ON e.id=n.effect_id
 WHERE o.state IN ('preparing','ready','committing','completed','unknown','interrupted')
 AND e.state IN ('prepared','submitting','confirmed','unknown')
 AND NOT EXISTS(SELECT 1 FROM native_consultation_returns r JOIN operation_effects f ON f.id=r.effect_id
 WHERE r.consultation_record_id=l.operation_record_id AND r.applied_json IS NOT NULL AND f.state='confirmed')
 AND NOT EXISTS(SELECT 1 FROM native_private_lifetimes v WHERE v.operation_record_id=l.operation_record_id AND v.restored_at IS NOT NULL);
PRAGMA user_version=51;
