-- Immutable logical offer and server-issued Core mapping. No transport lease,
-- negotiated codec, active stream or readiness is manufactured by this row.
CREATE TABLE connection_offers (
 connection_id TEXT PRIMARY KEY REFERENCES connections(id),
 runtime_connection_id TEXT NOT NULL UNIQUE,
 operation_id TEXT NOT NULL,
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 offer_json TEXT NOT NULL CHECK(json_valid(offer_json)),
 created_at INTEGER NOT NULL
);
CREATE TRIGGER connection_offer_immutable BEFORE UPDATE ON connection_offers
 BEGIN SELECT RAISE(ABORT,'connection offer is immutable'); END;

-- Bind the nested native intent to the same actor/epoch receipt ledger.
DROP TRIGGER control_receipt_binding;
CREATE TRIGGER control_receipt_binding BEFORE INSERT ON operation_control_receipts
 WHEN NOT EXISTS(SELECT 1 FROM operations o WHERE o.record_id=NEW.operation_record_id
 AND o.ledger_version=0 AND o.state IN ('completed','failed') AND o.intent_json IS NOT NULL
 AND CASE WHEN json_extract(o.intent_json,'$.op')='connection_prepare' THEN json_extract(o.intent_json,'$.request.preparation_id') ELSE COALESCE(json_extract(o.intent_json,'$.request_id'),json_extract(o.intent_json,'$.request.operation_id')) END=o.operation_id
 AND (json_extract(o.intent_json,'$.op')!='connection_prepare' OR (json_extract(o.intent_json,'$.request.intent.conversation_id')=o.conversation_id AND json_extract(o.intent_json,'$.request.intent.session_id')=o.session_id AND (o.connection_id IS NULL OR json_extract(o.intent_json,'$.request.intent.connection_id')=o.connection_id)))
 AND (json_extract(o.intent_json,'$.op')!='membership' OR json_extract(o.intent_json,'$.request.conversation_id')=o.conversation_id)
 AND (
 (o.conversation_id IS NULL AND NEW.actor_participant_id IS NULL AND NEW.actor_authority='unbound')
 OR (o.conversation_id IS NOT NULL AND NEW.actor_authority='member' AND EXISTS(SELECT 1 FROM participants p
 WHERE p.participant_id=NEW.actor_participant_id AND p.conversation_id=o.conversation_id
 AND p.identity_id=o.owner_id AND p.membership_epoch=NEW.actor_membership_epoch))
 OR (o.conversation_id IS NOT NULL AND NEW.actor_participant_id IS NULL AND json_extract(o.intent_json,'$.op')='membership'
 AND (NEW.actor_authority='administrator' OR (NEW.actor_authority='invitation'
 AND json_extract(o.intent_json,'$.request.change.action') IN ('join','decline')
 AND EXISTS(SELECT 1 FROM membership_invitations x WHERE x.id=json_extract(o.intent_json,'$.request.change.invitation_id')
 AND x.conversation_id=o.conversation_id AND x.identity_id=o.owner_id))))
 ))
 BEGIN SELECT RAISE(ABORT,'control receipt binding mismatch'); END;
PRAGMA user_version=36;
