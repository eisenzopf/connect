-- An ordinary pending offer is retired before any fresh replacement ticket.
-- This immutable attribution survives the later public Connection binding CAS.
CREATE TABLE native_replacement_candidates (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 candidate_connection_id TEXT NOT NULL UNIQUE REFERENCES connections(id),
 original_record_id TEXT NOT NULL UNIQUE REFERENCES native_offer_operations(operation_record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 runtime_epoch TEXT NOT NULL,
 original_digest TEXT NOT NULL,
 retirement_effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id),
 claim_json TEXT NOT NULL CHECK(json_valid(claim_json) AND length(claim_json)<=8192)
);
CREATE TRIGGER native_candidate_claim_binding BEFORE INSERT ON native_replacement_candidates
 WHEN NOT EXISTS(SELECT 1 FROM operations parent
 JOIN credentials caller ON caller.id=NEW.credential_id AND caller.identity_id=parent.owner_id
 JOIN connections source ON source.id=parent.connection_id
 JOIN connections candidate ON candidate.id=NEW.candidate_connection_id
 JOIN connection_offers mapping ON mapping.connection_id=candidate.id
 JOIN native_offer_operations original ON original.operation_record_id=NEW.original_record_id
 JOIN operations root ON root.record_id=original.operation_record_id AND root.connection_id=candidate.id
 JOIN operation_effects original_effect ON original_effect.id=original.effect_id
 JOIN operation_effects retirement ON retirement.id=NEW.retirement_effect_id
 WHERE parent.record_id=NEW.operation_record_id AND parent.command_type='connection.replace'
 AND parent.state='accepted' AND parent.cancel_requested=0 AND parent.digest=NEW.original_digest
 AND caller.revoked=0 AND candidate.session_id=source.session_id
 AND candidate.participant_id=source.participant_id AND candidate.membership_epoch=source.membership_epoch
 AND candidate.identity_id=source.identity_id AND candidate.tenant_id=source.tenant_id
 AND candidate.endpoint_id!=source.endpoint_id AND candidate.state='preparing'
 AND candidate.generation=1 AND candidate.observed_runtime_id IS NULL AND candidate.readiness_json IS NULL
 AND root.command_type='connection.offer' AND root.state='preparing' AND root.cancel_requested=0
 AND root.lease_owner=original.attempt_id AND root.worker_runtime=NEW.runtime_epoch
 AND root.lease_expires_at=root.expires_at AND original.attempt_runtime=NEW.runtime_epoch
 AND original.effect_started=1 AND original.generation=candidate.generation
 AND original_effect.operation_record_id=root.record_id AND original_effect.provider='native-media'
 AND original_effect.state='submitting' AND original_effect.attempt=1
 AND retirement.operation_record_id=parent.record_id AND retirement.provider='native-candidate-retire'
 AND retirement.correlation_key=parent.record_id AND retirement.state='prepared' AND retirement.attempt=0
 AND json_extract(NEW.claim_json,'$.record')=root.record_id
 AND json_extract(NEW.claim_json,'$.operation')=root.operation_id
 AND json_extract(NEW.claim_json,'$.preparation')=original.preparation_record_id
 AND json_extract(NEW.claim_json,'$.credential')=original.credential_id
 AND json_extract(NEW.claim_json,'$.key')=original.key_id
 AND json_extract(NEW.claim_json,'$.device')=original.device_id
 AND json_extract(NEW.claim_json,'$.endpoint')=original.endpoint_id
 AND json_extract(NEW.claim_json,'$.attempt')=original.attempt_id
 AND json_extract(NEW.claim_json,'$.effect')=original.effect_id
 AND json_extract(NEW.claim_json,'$.runtime')=original.attempt_runtime
 AND json_extract(NEW.claim_json,'$.core')=mapping.runtime_connection_id
 AND json_extract(NEW.claim_json,'$.generation')=original.generation
 AND json_extract(NEW.claim_json,'$.expires')=root.expires_at
 AND mapping.operation_id=root.operation_id AND mapping.credential_id=original.credential_id)
 BEGIN SELECT RAISE(ABORT,'native candidate claim mismatch'); END;
CREATE TRIGGER native_candidate_claim_immutable BEFORE UPDATE ON native_replacement_candidates
 BEGIN SELECT RAISE(ABORT,'native candidate claim is immutable'); END;
CREATE TRIGGER native_candidate_claim_retained BEFORE DELETE ON native_replacement_candidates
 BEGIN SELECT RAISE(ABORT,'native candidate claim remains reserved'); END;

-- This view exposes attributed retirement metadata, not permission to prepare.
-- A fresh ticket separately requires the retirement effect to be confirmed.
CREATE VIEW native_candidate_retired_slots AS
 SELECT q.* FROM native_replacement_candidates q
 JOIN connections candidate ON candidate.id=q.candidate_connection_id
 JOIN connection_offers mapping ON mapping.connection_id=candidate.id
 JOIN operations original ON original.record_id=q.original_record_id
 JOIN native_offer_operations offer ON offer.operation_record_id=original.record_id
 JOIN operation_effects effect ON effect.id=offer.effect_id
 WHERE candidate.state='retired' AND candidate.observed_runtime_id IS NULL AND candidate.readiness_json IS NULL
 AND candidate.generation=json_extract(q.claim_json,'$.generation')+1
 AND mapping.runtime_connection_id=json_extract(q.claim_json,'$.core')
 AND offer.attempt_id=json_extract(q.claim_json,'$.attempt') AND offer.effect_id=json_extract(q.claim_json,'$.effect')
 AND offer.attempt_runtime=json_extract(q.claim_json,'$.runtime') AND offer.generation=json_extract(q.claim_json,'$.generation')
 AND ((original.state IN ('failed','cancelled') AND effect.state='failed') OR
 (original.state='completed' AND effect.state='confirmed' AND EXISTS(
 SELECT 1 FROM native_connection_lifetimes lifetime WHERE lifetime.operation_record_id=original.record_id AND lifetime.retired=1)));

-- Physical commit is already recorded before projection. These guards allow
-- only its exact claimed transfer; arbitrary owner/mapping changes still fail.
CREATE VIEW native_candidate_transfer_authority AS
 SELECT q.*,n.candidate_runtime_id,n.generation AS replacement_generation,
 n.runtime_epoch AS replacement_epoch,o.connection_id AS source_connection_id,
 allocation.observation_json AS target_json
 FROM native_replacement_candidates q
 JOIN operation_effects retired ON retired.id=q.retirement_effect_id
 AND retired.operation_record_id=q.operation_record_id AND retired.provider='native-candidate-retire'
 AND retired.state='confirmed' AND retired.attempt=1
 JOIN native_replacements n ON n.operation_record_id=q.operation_record_id
 AND n.candidate_connection_id=q.candidate_connection_id AND n.credential_id=q.credential_id
 AND n.runtime_epoch=q.runtime_epoch AND n.original_digest=q.original_digest
 JOIN operations o ON o.record_id=n.operation_record_id AND o.state IN ('preparing','unknown')
 JOIN native_replacement_steps committed ON committed.operation_record_id=n.operation_record_id
 AND committed.phase='commit' AND committed.started=1 AND committed.observation_json IS NOT NULL
 JOIN operation_effects effect ON effect.id=committed.effect_id AND effect.state='confirmed'
 JOIN native_replacement_steps allocated ON allocated.operation_record_id=n.operation_record_id
 AND allocated.phase='allocate' AND allocated.started=1
 JOIN operation_effects allocation ON allocation.id=allocated.effect_id AND allocation.state='confirmed'
 WHERE json_extract(retired.observation_json,'$.core')=json_extract(q.claim_json,'$.core')
 AND json_extract(retired.observation_json,'$.generation')=json_extract(q.claim_json,'$.generation')
 AND json_extract(retired.observation_json,'$.retired_generation')=json_extract(q.claim_json,'$.generation')+1;

DROP TRIGGER connection_owner_immutable;
CREATE TRIGGER connection_owner_immutable BEFORE UPDATE ON connections
 WHEN NEW.id!=OLD.id OR NEW.session_id!=OLD.session_id OR NEW.participant_id!=OLD.participant_id
 OR NEW.membership_epoch!=OLD.membership_epoch OR NEW.identity_id!=OLD.identity_id
 OR NEW.tenant_id!=OLD.tenant_id OR NEW.endpoint_id!=OLD.endpoint_id OR NEW.generation<OLD.generation
 OR (NEW.replaces IS NOT OLD.replaces AND NOT EXISTS(
 SELECT 1 FROM native_candidate_transfer_authority transfer WHERE transfer.candidate_connection_id=OLD.id
 AND OLD.state='retired' AND OLD.observed_runtime_id IS NULL AND OLD.readiness_json IS NULL
 AND OLD.generation=json_extract(transfer.claim_json,'$.generation')+1
 AND NEW.replaces=transfer.source_connection_id AND NEW.generation=transfer.replacement_generation
 AND NEW.generation>json_extract(transfer.claim_json,'$.generation') AND NEW.state='ready'
 AND NEW.observed_runtime_id=transfer.replacement_epoch AND NEW.readiness_json IS NOT NULL))
 BEGIN SELECT RAISE(ABORT,'connection ownership is immutable'); END;
DROP TRIGGER connection_offer_immutable;
CREATE TRIGGER connection_offer_immutable BEFORE UPDATE ON connection_offers
 WHEN NOT EXISTS(SELECT 1 FROM native_candidate_transfer_authority transfer
 JOIN connections candidate ON candidate.id=transfer.candidate_connection_id
 WHERE OLD.connection_id=transfer.candidate_connection_id AND NEW.connection_id=OLD.connection_id
 AND OLD.runtime_connection_id=json_extract(transfer.claim_json,'$.core')
 AND OLD.operation_id=json_extract(transfer.claim_json,'$.operation')
 AND OLD.credential_id=json_extract(transfer.claim_json,'$.credential')
 AND NEW.runtime_connection_id=transfer.candidate_runtime_id
 AND NEW.operation_id=json_extract(transfer.target_json,'$.intent.operation_id')
 AND NEW.credential_id=json_extract(transfer.target_json,'$.credential_id')
 AND json(NEW.offer_json)=json_remove(json_extract(transfer.target_json,'$.intent.offer'),'$.replacement_preparation')
 AND candidate.replaces=transfer.source_connection_id AND candidate.state='ready'
 AND candidate.generation=transfer.replacement_generation
 AND candidate.observed_runtime_id=transfer.replacement_epoch AND candidate.readiness_json IS NOT NULL)
 BEGIN SELECT RAISE(ABORT,'connection offer is immutable'); END;
PRAGMA user_version=114;
