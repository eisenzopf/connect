-- A retained local rejection is terminal and has no physical Connection binding.
-- Its asserted target remains only in the immutable wire intent.
CREATE TABLE native_offer_rejections (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 preparation_record_id TEXT NOT NULL UNIQUE REFERENCES operation_control_receipts(operation_record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 key_id TEXT NOT NULL REFERENCES peer_signing_keys(key_id),
 device_id TEXT NOT NULL REFERENCES devices(id),
 source_expires_at INTEGER NOT NULL
);
CREATE TRIGGER native_offer_rejection_binding BEFORE INSERT ON native_offer_rejections
 WHEN NOT EXISTS(
 SELECT 1 FROM operations wire
 JOIN operation_control_receipts receipt ON receipt.operation_record_id=NEW.preparation_record_id
 JOIN operations preparation ON preparation.record_id=receipt.operation_record_id
 JOIN credentials credential ON credential.id=NEW.credential_id
 JOIN credential_signing_keys binding ON binding.credential_id=credential.id AND binding.key_id=NEW.key_id
 JOIN peer_signing_keys key ON key.key_id=binding.key_id
 WHERE wire.record_id=NEW.operation_record_id AND wire.ledger_version=0
 AND wire.command_type='connection.offer' AND wire.state='failed' AND wire.connection_id IS NULL
 AND wire.result_json IS NULL AND wire.problem_json IS NOT NULL
 AND preparation.command_type='connection.prepare' AND preparation.state='failed'
 AND preparation.result_json IS NULL AND preparation.problem_json=wire.problem_json
 AND wire.owner_id=preparation.owner_id AND wire.tenant_id=preparation.tenant_id
 AND wire.conversation_id=preparation.conversation_id AND wire.session_id=preparation.session_id
 AND wire.expected_revision IS preparation.expected_revision
 AND json_extract(preparation.intent_json,'$.request.intent')=wire.intent_json
 AND json_extract(wire.intent_json,'$.operation_id')=wire.operation_id
 AND json_extract(wire.intent_json,'$.conversation_id')=wire.conversation_id
 AND json_extract(wire.intent_json,'$.session_id')=wire.session_id
 AND receipt.actor_authority='member' AND receipt.actor_participant_id IS NOT NULL
 AND receipt.actor_membership_epoch IS NOT NULL
 AND credential.identity_id=wire.owner_id AND key.identity_id=wire.owner_id
 AND key.device_id=NEW.device_id AND key.not_before<=wire.created_at
 AND key.expires_at>=NEW.source_expires_at AND credential.expires_at>=NEW.source_expires_at
 AND wire.expires_at<=NEW.source_expires_at
 AND NOT EXISTS(SELECT 1 FROM operation_effects e WHERE e.operation_record_id=wire.record_id)
 AND NOT EXISTS(SELECT 1 FROM native_offer_operations n WHERE n.operation_record_id=wire.record_id)
 ) BEGIN SELECT RAISE(ABORT,'native rejection binding mismatch'); END;
CREATE TRIGGER native_offer_rejection_immutable BEFORE UPDATE ON native_offer_rejections
 BEGIN SELECT RAISE(ABORT,'native rejection origin is immutable'); END;
CREATE TRIGGER native_offer_rejection_retained BEFORE DELETE ON native_offer_rejections
 BEGIN SELECT RAISE(ABORT,'native rejection identity remains reserved'); END;
CREATE TRIGGER native_offer_rejection_no_effect BEFORE INSERT ON operation_effects
 WHEN EXISTS(SELECT 1 FROM native_offer_rejections n WHERE n.operation_record_id=NEW.operation_record_id)
 BEGIN SELECT RAISE(ABORT,'native rejection cannot start an effect'); END;
CREATE TRIGGER native_offer_rejection_record_immutable BEFORE UPDATE ON operations
 WHEN EXISTS(SELECT 1 FROM native_offer_rejections n WHERE n.operation_record_id=OLD.record_id)
 AND (NEW.state!=OLD.state OR NEW.result_json IS NOT OLD.result_json
 OR NEW.problem_json IS NOT OLD.problem_json OR NEW.expires_at IS NOT OLD.expires_at
 OR NEW.ledger_version!=OLD.ledger_version OR NEW.cancel_requested!=OLD.cancel_requested)
 BEGIN SELECT RAISE(ABORT,'native rejection result is immutable'); END;
PRAGMA user_version=38;
