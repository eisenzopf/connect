-- Exact native plans and source incarnations precede possible consultation IO.
CREATE TABLE native_consultation_attempts (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 attempt_id TEXT NOT NULL UNIQUE,
 runtime_epoch TEXT NOT NULL,
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id),
 source_expires_at INTEGER NOT NULL,
 selection_json TEXT NOT NULL CHECK(json_valid(selection_json) AND length(selection_json)<=32768),
 plan_json TEXT NOT NULL CHECK(json_valid(plan_json) AND json_type(plan_json)='object' AND length(plan_json)<=32768),
 plan_digest TEXT NOT NULL CHECK(length(plan_digest)=64),
 started INTEGER NOT NULL DEFAULT 0 CHECK(started IN (0,1)),
 applied_json TEXT CHECK(applied_json IS NULL OR (started=1 AND json_valid(applied_json) AND length(applied_json)<=32768))
);
CREATE TRIGGER native_consultation_attempt_binding BEFORE INSERT ON native_consultation_attempts
 WHEN NOT EXISTS(SELECT 1 FROM operations o JOIN credentials k ON k.id=NEW.credential_id
 JOIN operation_effects e ON e.id=NEW.effect_id AND e.operation_record_id=o.record_id
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='session.consult' AND o.ledger_version=1
 AND o.state='preparing' AND k.identity_id=o.owner_id AND k.revoked=0
 AND e.provider='native-consultation' AND e.correlation_key=o.record_id
 AND e.state='prepared' AND e.attempt=0 AND o.expires_at>=NEW.source_expires_at
 AND json_extract(NEW.plan_json,'$.operation_id')=o.record_id)
 BEGIN SELECT RAISE(ABORT,'native consultation attempt binding mismatch'); END;
CREATE TRIGGER native_consultation_attempt_immutable BEFORE UPDATE ON native_consultation_attempts
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.attempt_id!=OLD.attempt_id
 OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.credential_id!=OLD.credential_id
 OR NEW.effect_id!=OLD.effect_id OR NEW.source_expires_at!=OLD.source_expires_at
 OR NEW.selection_json!=OLD.selection_json OR NEW.plan_json!=OLD.plan_json OR NEW.plan_digest!=OLD.plan_digest
 OR NEW.started<OLD.started OR (OLD.applied_json IS NOT NULL AND NEW.applied_json IS NOT OLD.applied_json)
 BEGIN SELECT RAISE(ABORT,'native consultation attempt is immutable'); END;
CREATE TRIGGER native_consultation_attempt_retained BEFORE DELETE ON native_consultation_attempts
 BEGIN SELECT RAISE(ABORT,'native consultation attempt remains reserved'); END;
PRAGMA user_version=46;
