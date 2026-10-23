-- New native custody boundary; historical schema versions require explicit reset.
CREATE TABLE native_handoffs (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 runtime_epoch TEXT NOT NULL,
 original_digest TEXT NOT NULL,
 attempt_id TEXT NOT NULL UNIQUE,
 effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id),
 plan_json TEXT NOT NULL CHECK(json_valid(plan_json) AND length(plan_json)<=32768),
 selection_json TEXT NOT NULL CHECK(json_valid(selection_json) AND length(selection_json)<=32768),
 plan_digest TEXT NOT NULL,
 started INTEGER NOT NULL DEFAULT 0 CHECK(started IN (0,1)),
 applied_json TEXT CHECK(applied_json IS NULL OR (started=1 AND json_valid(applied_json) AND length(applied_json)<=32768))
);
CREATE TRIGGER native_handoff_binding BEFORE INSERT ON native_handoffs
 WHEN NOT EXISTS(SELECT 1 FROM operations o
 JOIN credentials k ON k.id=NEW.credential_id AND k.identity_id=o.owner_id AND k.revoked=0
 JOIN operation_effects e ON e.id=NEW.effect_id AND e.operation_record_id=o.record_id
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='session.handoff'
 AND o.state='preparing' AND o.digest=NEW.original_digest AND o.lease_owner=NEW.attempt_id
 AND e.provider='native-handoff' AND e.state='prepared' AND e.attempt=0)
 BEGIN SELECT RAISE(ABORT,'native handoff binding mismatch'); END;
CREATE TRIGGER native_handoff_immutable BEFORE UPDATE ON native_handoffs
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.credential_id!=OLD.credential_id
 OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.original_digest!=OLD.original_digest
 OR NEW.attempt_id!=OLD.attempt_id OR NEW.effect_id!=OLD.effect_id
 OR NEW.plan_json!=OLD.plan_json OR NEW.selection_json!=OLD.selection_json OR NEW.plan_digest!=OLD.plan_digest
 OR NEW.started<OLD.started OR (OLD.applied_json IS NOT NULL AND NEW.applied_json IS NOT OLD.applied_json)
 BEGIN SELECT RAISE(ABORT,'native handoff is immutable'); END;
CREATE TRIGGER native_handoff_retained BEFORE DELETE ON native_handoffs
 BEGIN SELECT RAISE(ABORT,'native handoff remains reserved'); END;
PRAGMA user_version=54;
