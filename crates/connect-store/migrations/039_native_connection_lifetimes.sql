-- Successful negotiation keeps separate room for actual later local cleanup.
CREATE TABLE native_connection_lifetimes (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 connection_id TEXT NOT NULL UNIQUE REFERENCES connections(id),
 runtime_epoch TEXT NOT NULL,
 generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
 retired INTEGER NOT NULL DEFAULT 0 CHECK(retired IN (0,1))
);
CREATE TRIGGER native_lifetime_binding BEFORE INSERT ON native_connection_lifetimes
 WHEN NOT EXISTS(SELECT 1 FROM operations o JOIN native_offer_operations n ON n.operation_record_id=o.record_id
 JOIN connections c ON c.id=o.connection_id WHERE o.record_id=NEW.operation_record_id
 AND o.state='completed' AND n.effect_started=1 AND n.attempt_runtime=NEW.runtime_epoch
 AND c.id=NEW.connection_id AND c.generation=NEW.generation AND c.state='ready'
 AND c.observed_runtime_id=NEW.runtime_epoch AND c.readiness_json IS NOT NULL)
 BEGIN SELECT RAISE(ABORT,'native lifetime binding mismatch'); END;
CREATE TRIGGER native_lifetime_immutable BEFORE UPDATE ON native_connection_lifetimes
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.connection_id!=OLD.connection_id
 OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.generation!=OLD.generation OR NEW.retired<OLD.retired
 BEGIN SELECT RAISE(ABORT,'native lifetime origin is immutable'); END;
CREATE TRIGGER native_lifetime_retained BEFORE DELETE ON native_connection_lifetimes
 BEGIN SELECT RAISE(ABORT,'native lifetime identity remains reserved'); END;
PRAGMA user_version=39;
