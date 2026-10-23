-- The wire exchange is an asynchronous class in the shared operation ledger.
-- Its completed private preparation is not completion of this operation.
CREATE TABLE native_offer_operations (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 preparation_record_id TEXT NOT NULL UNIQUE REFERENCES operation_control_receipts(operation_record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 key_id TEXT NOT NULL REFERENCES peer_signing_keys(key_id),
 device_id TEXT NOT NULL REFERENCES devices(id),
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
 resources_json TEXT NOT NULL CHECK(json_valid(resources_json) AND json_type(resources_json)='array'),
 source_expires_at INTEGER NOT NULL,
 terminal_reserve INTEGER NOT NULL CHECK(terminal_reserve BETWEEN 1 AND 257),
 effect_started INTEGER NOT NULL DEFAULT 0 CHECK(effect_started IN (0,1)),
 effect_id TEXT UNIQUE REFERENCES operation_effects(id),
 attempt_id TEXT,
 attempt_runtime TEXT,
 CHECK((effect_started=0 AND effect_id IS NULL AND attempt_id IS NULL AND attempt_runtime IS NULL)
    OR (effect_started=1 AND effect_id IS NOT NULL AND attempt_id IS NOT NULL AND attempt_runtime IS NOT NULL))
);
CREATE TRIGGER native_offer_binding BEFORE INSERT ON native_offer_operations
 WHEN NOT EXISTS(
 SELECT 1 FROM operations wire
 JOIN operation_control_receipts receipt ON receipt.operation_record_id=NEW.preparation_record_id
 JOIN operations preparation ON preparation.record_id=receipt.operation_record_id
 JOIN connections n ON n.id=wire.connection_id
 JOIN connection_offers offer ON offer.connection_id=n.id
 JOIN credentials credential ON credential.id=NEW.credential_id
 JOIN credential_signing_keys binding ON binding.credential_id=credential.id AND binding.key_id=NEW.key_id
 JOIN peer_signing_keys key ON key.key_id=binding.key_id
 WHERE wire.record_id=NEW.operation_record_id AND wire.ledger_version=0 AND wire.state='accepted'
 AND preparation.state='completed' AND preparation.command_type='connection.prepare'
 AND wire.command_type='connection.offer' AND wire.owner_id=preparation.owner_id
 AND wire.conversation_id=preparation.conversation_id AND wire.session_id=preparation.session_id
 AND wire.connection_id=preparation.connection_id AND wire.operation_id=offer.operation_id
 AND json_extract(wire.intent_json,'$.operation_id')=wire.operation_id
 AND json_extract(wire.intent_json,'$.conversation_id')=wire.conversation_id
 AND json_extract(wire.intent_json,'$.session_id')=wire.session_id
 AND json_extract(wire.intent_json,'$.connection_id')=wire.connection_id
 AND json_extract(wire.intent_json,'$.offer')=offer.offer_json
 AND wire.expected_revision IS json_extract(wire.intent_json,'$.expected_revision')
 AND receipt.actor_authority='member' AND receipt.actor_participant_id=NEW.participant_id
 AND receipt.actor_membership_epoch=NEW.membership_epoch AND receipt.resources_json=NEW.resources_json
 AND n.participant_id=NEW.participant_id AND n.membership_epoch=NEW.membership_epoch
 AND n.endpoint_id=NEW.endpoint_id AND n.generation=NEW.generation AND n.state='preparing'
 AND n.observed_runtime_id IS NULL AND n.readiness_json IS NULL
 AND credential.identity_id=wire.owner_id AND key.identity_id=wire.owner_id
 AND key.device_id=NEW.device_id AND key.not_before<=wire.created_at AND key.expires_at>=NEW.source_expires_at
 AND credential.expires_at>=NEW.source_expires_at AND wire.expires_at<=NEW.source_expires_at
 AND NEW.terminal_reserve=json_array_length(NEW.resources_json)+1
 ) BEGIN SELECT RAISE(ABORT,'native offer binding mismatch'); END;
CREATE TRIGGER native_offer_origin_immutable BEFORE UPDATE ON native_offer_operations
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.preparation_record_id!=OLD.preparation_record_id
 OR NEW.credential_id!=OLD.credential_id OR NEW.key_id!=OLD.key_id OR NEW.device_id!=OLD.device_id
 OR NEW.endpoint_id!=OLD.endpoint_id OR NEW.participant_id!=OLD.participant_id
 OR NEW.membership_epoch!=OLD.membership_epoch OR NEW.generation!=OLD.generation
 OR NEW.resources_json!=OLD.resources_json OR NEW.source_expires_at!=OLD.source_expires_at
 OR NEW.terminal_reserve!=OLD.terminal_reserve OR NEW.effect_started<OLD.effect_started
 OR (OLD.effect_started=1 AND (NEW.effect_id IS NOT OLD.effect_id OR NEW.attempt_id IS NOT OLD.attempt_id OR NEW.attempt_runtime IS NOT OLD.attempt_runtime))
 BEGIN SELECT RAISE(ABORT,'native offer origin is immutable'); END;
CREATE TRIGGER native_offer_attempt_binding BEFORE UPDATE ON native_offer_operations
 WHEN NEW.effect_started=1 AND NOT EXISTS(SELECT 1 FROM operation_effects e
 WHERE e.id=NEW.effect_id AND e.operation_record_id=NEW.operation_record_id
 AND e.provider='native-media' AND e.idempotency_mode='none' AND e.submission_generation=1)
 BEGIN SELECT RAISE(ABORT,'native offer attempt binding mismatch'); END;
CREATE TRIGGER native_offer_retained BEFORE DELETE ON native_offer_operations
 BEGIN SELECT RAISE(ABORT,'native offer identity remains reserved'); END;
CREATE TRIGGER native_offer_record_immutable BEFORE UPDATE ON operations
 WHEN EXISTS(SELECT 1 FROM native_offer_operations n WHERE n.operation_record_id=OLD.record_id)
 AND (NEW.expires_at IS NOT OLD.expires_at OR NEW.ledger_version!=OLD.ledger_version
 OR (OLD.state IN ('completed','cancelled','failed') AND (NEW.state!=OLD.state OR NEW.result_json IS NOT OLD.result_json OR NEW.problem_json IS NOT OLD.problem_json)))
 BEGIN SELECT RAISE(ABORT,'native offer record is immutable'); END;
PRAGMA user_version=37;
