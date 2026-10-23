-- Telephone commit custody uses the real shared reservation step, never a native peer.
CREATE TABLE telephone_replacement_commits (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_replacements(operation_record_id),
 runtime_epoch TEXT NOT NULL,
 attempt_id TEXT NOT NULL UNIQUE,
 effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id),
 descriptor_json TEXT NOT NULL CHECK(json_valid(descriptor_json) AND length(descriptor_json)<=262144),
 observation_json TEXT CHECK(observation_json IS NULL OR (json_valid(observation_json) AND length(observation_json)<=32768)),
 audience_json TEXT CHECK(audience_json IS NULL OR (json_valid(audience_json) AND length(audience_json)<=262144))
);
CREATE TRIGGER telephone_replacement_commit_binding BEFORE INSERT ON telephone_replacement_commits
 WHEN NOT EXISTS(SELECT 1 FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id
 JOIN native_replacement_steps s ON s.operation_record_id=n.operation_record_id AND s.phase='commit'
 JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=o.record_id
 WHERE n.operation_record_id=NEW.operation_record_id AND n.runtime_epoch=NEW.runtime_epoch
 AND o.state='preparing' AND o.cancel_requested=0 AND s.started=1 AND s.attempt_id=NEW.attempt_id AND s.effect_id=NEW.effect_id
 AND e.state='submitting' AND e.attempt=1 AND e.submission_generation=1
 AND (SELECT count(*) FROM telephone_replacement_phases p JOIN operation_effects pe ON pe.id=p.effect_id WHERE p.operation_record_id=n.operation_record_id AND p.runtime_epoch=n.runtime_epoch AND p.observation_json IS NOT NULL AND pe.state='confirmed')=3)
 BEGIN SELECT RAISE(ABORT,'telephone commit binding mismatch'); END;
CREATE TRIGGER telephone_replacement_commit_immutable BEFORE UPDATE ON telephone_replacement_commits
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.attempt_id!=OLD.attempt_id OR NEW.effect_id!=OLD.effect_id
 OR NEW.descriptor_json!=OLD.descriptor_json OR (OLD.observation_json IS NOT NULL AND NEW.observation_json IS NOT OLD.observation_json)
 OR (OLD.audience_json IS NOT NULL AND NEW.audience_json IS NOT OLD.audience_json)
 BEGIN SELECT RAISE(ABORT,'telephone commit capture is immutable'); END;
CREATE TRIGGER telephone_replacement_commit_retained BEFORE DELETE ON telephone_replacement_commits
 BEGIN SELECT RAISE(ABORT,'telephone commit remains reserved'); END;
CREATE TABLE telephone_connection_lifetimes (
 operation_record_id TEXT PRIMARY KEY REFERENCES telephone_replacement_commits(operation_record_id),
 connection_id TEXT NOT NULL UNIQUE REFERENCES connections(id),
 runtime_epoch TEXT NOT NULL,
 generation INTEGER NOT NULL CHECK(generation BETWEEN 2 AND 9007199254740991),
 retired INTEGER NOT NULL DEFAULT 0 CHECK(retired IN (0,1))
);
CREATE TRIGGER telephone_connection_lifetime_binding BEFORE INSERT ON telephone_connection_lifetimes
 WHEN NOT EXISTS(SELECT 1 FROM telephone_replacement_commits t JOIN native_replacements n ON n.operation_record_id=t.operation_record_id
 JOIN operations o ON o.record_id=n.operation_record_id JOIN operation_effects e ON e.id=t.effect_id
 JOIN connections c ON c.id=n.candidate_connection_id
 WHERE t.operation_record_id=NEW.operation_record_id AND t.runtime_epoch=NEW.runtime_epoch AND NEW.runtime_epoch=n.runtime_epoch
 AND e.state='confirmed' AND t.observation_json IS NOT NULL AND t.audience_json IS NOT NULL AND o.state='completed'
 AND c.id=NEW.connection_id AND c.generation=NEW.generation AND c.generation=n.generation AND c.observed_runtime_id=NEW.runtime_epoch
 AND c.state='ready' AND NEW.retired=0)
 BEGIN SELECT RAISE(ABORT,'telephone lifetime binding mismatch'); END;
CREATE TRIGGER telephone_connection_lifetime_immutable BEFORE UPDATE ON telephone_connection_lifetimes
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.connection_id!=OLD.connection_id OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.generation!=OLD.generation OR NEW.retired<OLD.retired
 BEGIN SELECT RAISE(ABORT,'telephone lifetime is monotonic'); END;
CREATE TRIGGER telephone_connection_lifetime_retained BEFORE DELETE ON telephone_connection_lifetimes
 BEGIN SELECT RAISE(ABORT,'telephone lifetime remains reserved'); END;
PRAGMA user_version=61;
