-- Fresh transition-ledger boundary, no in-place upgrade of candidate state.
ALTER TABLE operations ADD COLUMN ledger_version INTEGER NOT NULL DEFAULT 0 CHECK(ledger_version IN (0,1));
ALTER TABLE operations ADD COLUMN worker_runtime TEXT;
ALTER TABLE operations ADD COLUMN worker_cleanup INTEGER NOT NULL DEFAULT 0 CHECK(worker_cleanup IN (0,1));
ALTER TABLE operations ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0,1));
ALTER TABLE operation_effects ADD COLUMN submission_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE operation_effects ADD COLUMN compensates_effect_id TEXT REFERENCES operation_effects(id);
ALTER TABLE conversations ADD COLUMN closed_at INTEGER;
CREATE TRIGGER conversation_close_time AFTER UPDATE OF closed ON conversations WHEN NEW.closed=1 AND OLD.closed=0 BEGIN UPDATE conversations SET closed_at=strftime('%s','now') WHERE id=NEW.id; END;
CREATE TABLE operation_members (
 operation_record_id TEXT NOT NULL REFERENCES operations(record_id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 identity_id TEXT NOT NULL REFERENCES identities(id),
 membership_epoch INTEGER NOT NULL,
 PRIMARY KEY(operation_record_id,participant_id)
);
CREATE TABLE operation_connections (
 operation_record_id TEXT NOT NULL REFERENCES operations(record_id),
 connection_id TEXT NOT NULL REFERENCES connections(id),
 generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
 PRIMARY KEY(operation_record_id,connection_id)
);
CREATE TABLE operation_endpoints (
 operation_record_id TEXT NOT NULL REFERENCES operations(record_id),
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 revision INTEGER NOT NULL,
 PRIMARY KEY(operation_record_id,endpoint_id)
);
CREATE TABLE operation_decisions (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 target_record_id TEXT NOT NULL REFERENCES operations(record_id),
 action TEXT NOT NULL CHECK(action IN ('cancel','reconcile','cleanup'))
);
CREATE TABLE operation_evidence (
 observation_id TEXT PRIMARY KEY REFERENCES operation_observations(id),
 operation_record_id TEXT NOT NULL REFERENCES operations(record_id),
 operation_revision INTEGER NOT NULL,
 worker_generation INTEGER NOT NULL,
 outcome TEXT NOT NULL CHECK(outcome IN ('completed','cancelled','failed')),
 source_kind TEXT NOT NULL CHECK(source_kind IN ('local-state','provider-status','operator-attestation')),
 completion_json TEXT CHECK(completion_json IS NULL OR json_valid(completion_json))
);
CREATE TRIGGER operation_state_edges BEFORE UPDATE OF state ON operations
 WHEN OLD.ledger_version=1 AND NEW.state!=OLD.state AND NOT(
 (OLD.state='accepted' AND NEW.state IN ('preparing','cancelled','failed','interrupted')) OR
 (OLD.state='preparing' AND NEW.state IN ('ready','committing','cancelled','failed','interrupted','unknown')) OR
 (OLD.state='ready' AND NEW.state IN ('committing','cancelled','failed','interrupted')) OR
 (OLD.state='committing' AND NEW.state IN ('completed','failed','interrupted','unknown')) OR
 (OLD.state='interrupted' AND NEW.state IN ('preparing','completed','cancelled','failed','unknown')) OR
 (OLD.state='unknown' AND NEW.state IN ('completed','cancelled','failed')))
 BEGIN SELECT RAISE(ABORT,'illegal operation state transition'); END;
CREATE TRIGGER operation_result_immutable BEFORE UPDATE ON operations
 WHEN OLD.ledger_version=1 AND (NEW.ledger_version!=OLD.ledger_version OR
 (OLD.state IN ('completed','cancelled','failed') AND NEW.state!=OLD.state) OR
 (NEW.result_json IS NOT OLD.result_json AND OLD.result_json IS NOT NULL AND NEW.tombstoned=0) OR
 NEW.revision<OLD.revision OR NEW.cancel_requested<OLD.cancel_requested)
 BEGIN SELECT RAISE(ABORT,'operation terminal evidence is immutable'); END;
PRAGMA user_version=25;
CREATE TRIGGER operation_member_binding BEFORE INSERT ON operation_members
 WHEN NOT EXISTS(SELECT 1 FROM operations o JOIN participants p ON p.conversation_id=o.conversation_id AND p.tenant_id=o.tenant_id WHERE o.record_id=NEW.operation_record_id AND p.participant_id=NEW.participant_id AND p.identity_id=NEW.identity_id AND p.membership_epoch=NEW.membership_epoch)
 BEGIN SELECT RAISE(ABORT,'operation membership scope mismatch'); END;
CREATE TRIGGER operation_connection_binding BEFORE INSERT ON operation_connections
 WHEN NOT EXISTS(SELECT 1 FROM operations o JOIN connections v ON v.session_id=o.session_id AND v.tenant_id=o.tenant_id WHERE o.record_id=NEW.operation_record_id AND v.id=NEW.connection_id AND v.generation=NEW.generation)
 BEGIN SELECT RAISE(ABORT,'operation connection scope mismatch'); END;
CREATE TRIGGER operation_member_immutable BEFORE UPDATE ON operation_members BEGIN SELECT RAISE(ABORT,'operation membership snapshot is immutable'); END;
CREATE TRIGGER operation_connection_immutable BEFORE UPDATE ON operation_connections BEGIN SELECT RAISE(ABORT,'operation connection snapshot is immutable'); END;
CREATE TRIGGER operation_endpoint_immutable BEFORE UPDATE ON operation_endpoints BEGIN SELECT RAISE(ABORT,'operation endpoint snapshot is immutable'); END;
CREATE TRIGGER operation_compensation_binding BEFORE INSERT ON operation_effects WHEN NEW.compensates_effect_id IS NOT NULL
 AND NOT EXISTS(SELECT 1 FROM operation_effects e WHERE e.id=NEW.compensates_effect_id AND e.operation_record_id=NEW.operation_record_id AND e.provider=NEW.provider AND e.compensates_effect_id IS NULL)
 BEGIN SELECT RAISE(ABORT,'compensation effect scope mismatch'); END;
CREATE TRIGGER operation_effect_immutable BEFORE UPDATE ON operation_effects
 WHEN EXISTS(SELECT 1 FROM operations o WHERE o.record_id=OLD.operation_record_id AND o.ledger_version=1) AND (
 NEW.id!=OLD.id OR NEW.operation_record_id!=OLD.operation_record_id OR NEW.provider!=OLD.provider OR NEW.correlation_key!=OLD.correlation_key OR NEW.idempotency_mode!=OLD.idempotency_mode OR NEW.compensates_effect_id IS NOT OLD.compensates_effect_id OR NEW.attempt<OLD.attempt OR NEW.attempt>1 OR (OLD.attempt>0 AND NEW.submission_generation!=OLD.submission_generation) OR (OLD.provider_ref IS NOT NULL AND NEW.provider_ref IS NOT OLD.provider_ref) OR (OLD.state IN ('confirmed','failed','cancelled') AND NEW.state!=OLD.state))
 BEGIN SELECT RAISE(ABORT,'effect identity or terminal evidence is immutable'); END;
CREATE TRIGGER operation_deadline_immutable BEFORE UPDATE ON operations
 WHEN NEW.ledger_version!=OLD.ledger_version OR (OLD.ledger_version=1 AND NEW.expires_at IS NOT OLD.expires_at)
 BEGIN SELECT RAISE(ABORT,'operation admission deadline is immutable'); END;
