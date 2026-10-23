-- Fresh schema only: ordinary and canonical mutations share one operation key
-- and result. Existing candidate databases are refused, never imported here.
DROP TABLE requests;
CREATE TABLE operation_control_receipts (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 resources_json TEXT NOT NULL CHECK(json_valid(resources_json) AND json_type(resources_json)='array'),
 task_id TEXT REFERENCES tasks(id),
 grant_id TEXT REFERENCES task_grants(id),
 actor_participant_id TEXT REFERENCES participants(participant_id),
 actor_membership_epoch INTEGER,
 CHECK(grant_id IS NULL OR task_id IS NOT NULL),
 CHECK((actor_participant_id IS NULL)=(actor_membership_epoch IS NULL))
);
CREATE TRIGGER control_receipt_binding BEFORE INSERT ON operation_control_receipts
 WHEN NOT EXISTS(SELECT 1 FROM operations o WHERE o.record_id=NEW.operation_record_id
 AND o.ledger_version=0 AND o.state IN ('completed','failed') AND o.intent_json IS NOT NULL
 AND json_extract(o.intent_json,'$.request_id')=o.operation_id
 AND (o.conversation_id IS NULL)=(NEW.actor_participant_id IS NULL)
 AND (NEW.actor_participant_id IS NULL OR EXISTS(SELECT 1 FROM participants p
 WHERE p.participant_id=NEW.actor_participant_id AND p.conversation_id=o.conversation_id
 AND p.identity_id=o.owner_id AND p.membership_epoch=NEW.actor_membership_epoch)))
 BEGIN SELECT RAISE(ABORT,'control receipt binding mismatch'); END;
CREATE TRIGGER control_receipt_immutable BEFORE UPDATE ON operation_control_receipts
 BEGIN SELECT RAISE(ABORT,'control receipt origin is immutable'); END;
CREATE TRIGGER control_receipt_retained BEFORE DELETE ON operation_control_receipts
 BEGIN SELECT RAISE(ABORT,'control receipt identity remains reserved'); END;
CREATE TRIGGER control_result_immutable BEFORE UPDATE ON operations
 WHEN EXISTS(SELECT 1 FROM operation_control_receipts x WHERE x.operation_record_id=OLD.record_id)
 AND (NEW.state!=OLD.state OR NEW.result_json IS NOT OLD.result_json OR NEW.problem_json IS NOT OLD.problem_json
 OR NEW.intent_json IS NOT OLD.intent_json OR NEW.tombstoned!=OLD.tombstoned OR NEW.updated_at!=OLD.updated_at
 OR NEW.revision!=OLD.revision OR NEW.cancel_requested!=OLD.cancel_requested)
 BEGIN SELECT RAISE(ABORT,'control mutation result is immutable'); END;
PRAGMA user_version=32;
