-- Fresh-state boundary: immutable event metadata uses the version at INSERT,
-- never a current resource lookup when an old cursor is replayed.
ALTER TABLE sessions ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0 CHECK(state_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE artifacts ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0 CHECK(state_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE voice_calls ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0 CHECK(state_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE sip_legs ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0 CHECK(state_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE context_deliveries ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 0 CHECK(state_revision BETWEEN 0 AND 9007199254740991);
CREATE TRIGGER session_state_revision AFTER UPDATE OF state,media,ended_reason,topology_revision,initiator_id,participant_id ON sessions
 WHEN NEW.state IS NOT OLD.state OR NEW.media IS NOT OLD.media OR NEW.ended_reason IS NOT OLD.ended_reason OR NEW.topology_revision!=OLD.topology_revision OR NEW.initiator_id!=OLD.initiator_id OR NEW.participant_id!=OLD.participant_id
 BEGIN UPDATE sessions SET state_revision=state_revision+1 WHERE id=NEW.id; END;
CREATE TRIGGER artifact_state_revision AFTER UPDATE OF content,state,blob_name,name,media_type,expected_size,expected_sha256,expires_at ON artifacts
 WHEN NEW.content IS NOT OLD.content OR NEW.state!=OLD.state OR NEW.blob_name IS NOT OLD.blob_name OR NEW.name!=OLD.name OR NEW.media_type!=OLD.media_type OR NEW.expected_size!=OLD.expected_size OR NEW.expected_sha256!=OLD.expected_sha256 OR NEW.expires_at IS NOT OLD.expires_at
 BEGIN UPDATE artifacts SET state_revision=state_revision+1 WHERE id=NEW.id; END;
CREATE TRIGGER call_state_revision AFTER UPDATE OF state,provider_id,ended_reason,end_requested,direction,route_id,destination ON voice_calls
 WHEN NEW.state!=OLD.state OR NEW.provider_id IS NOT OLD.provider_id OR NEW.ended_reason IS NOT OLD.ended_reason OR NEW.end_requested!=OLD.end_requested OR NEW.direction!=OLD.direction OR NEW.route_id!=OLD.route_id OR NEW.destination!=OLD.destination
 BEGIN UPDATE voice_calls SET state_revision=state_revision+1 WHERE id=NEW.id; END;
CREATE TRIGGER sip_state_revision AFTER UPDATE OF state,connection_id,ended_reason,end_requested,destination ON sip_legs
 WHEN NEW.state!=OLD.state OR NEW.connection_id IS NOT OLD.connection_id OR NEW.ended_reason IS NOT OLD.ended_reason OR NEW.end_requested!=OLD.end_requested OR NEW.destination!=OLD.destination
 BEGIN UPDATE sip_legs SET state_revision=state_revision+1 WHERE id=NEW.id; END;
CREATE TRIGGER context_state_revision AFTER UPDATE OF state,watermark,provider_ref,selection_json ON context_deliveries
 WHEN NEW.state!=OLD.state OR NEW.watermark!=OLD.watermark OR NEW.provider_ref IS NOT OLD.provider_ref OR NEW.selection_json!=OLD.selection_json
 BEGIN UPDATE context_deliveries SET state_revision=state_revision+1 WHERE id=NEW.id; END;
CREATE TRIGGER closed_conversation_revision AFTER UPDATE OF closed ON conversations WHEN NEW.closed!=OLD.closed AND NEW.revision=OLD.revision
 BEGIN UPDATE conversations SET revision=revision+1 WHERE id=NEW.id; END;
-- This context exists only within a mutation savepoint and is cleared before
-- its outer commit. It binds correlation to the authenticated writer, not a body.
CREATE TABLE journal_context (singleton INTEGER PRIMARY KEY CHECK(singleton=1),actor TEXT NOT NULL REFERENCES identities(id),operation_id TEXT NOT NULL);
ALTER TABLE journal ADD COLUMN source_json TEXT CHECK(source_json IS NULL OR json_valid(source_json));
CREATE TRIGGER journal_fact_metadata AFTER INSERT ON journal BEGIN
 UPDATE journal SET resource_revision=CASE
 WHEN json_extract(NEW.data_json,'$.receipt') IS NOT NULL THEN (SELECT COUNT(*) FROM message_receipts r WHERE r.message_id=json_extract(NEW.data_json,'$.receipt.message_id') AND r.participant_id=json_extract(NEW.data_json,'$.receipt.participant_id') AND r.membership_epoch=json_extract(NEW.data_json,'$.receipt.membership_epoch') AND r.channel=json_extract(NEW.data_json,'$.receipt.channel') AND r.delivery_id=json_extract(NEW.data_json,'$.receipt.delivery_id'))
 WHEN NEW.session_json IS NOT NULL THEN (SELECT state_revision FROM sessions WHERE id=json_extract(NEW.session_json,'$.id'))
 WHEN NEW.artifact_json IS NOT NULL THEN (SELECT state_revision FROM artifacts WHERE id=json_extract(NEW.artifact_json,'$.id'))
 WHEN NEW.call_json IS NOT NULL THEN (SELECT state_revision FROM voice_calls WHERE id=json_extract(NEW.call_json,'$.id'))
 WHEN NEW.message_id IS NOT NULL THEN 1
 WHEN json_extract(NEW.data_json,'$.source_kind')='session' AND NEW.operation_record_id IS NULL THEN (SELECT state_revision FROM sessions WHERE id=json_extract(NEW.data_json,'$.source_id'))
 WHEN json_extract(NEW.data_json,'$.source_kind')='sip_leg' THEN (SELECT state_revision FROM sip_legs WHERE id=json_extract(NEW.data_json,'$.source_id'))
 WHEN json_extract(NEW.data_json,'$.source_kind')='context_delivery' THEN (SELECT state_revision FROM context_deliveries WHERE id=json_extract(NEW.data_json,'$.source_id'))
 ELSE COALESCE(NEW.resource_revision,(SELECT revision FROM conversations WHERE id=NEW.conversation_id)) END,
 source_json=COALESCE(NEW.source_json,json_object('kind',CASE
 WHEN json_extract(NEW.data_json,'$.receipt') IS NOT NULL THEN json_extract(NEW.data_json,'$.receipt.provenance')
 WHEN NEW.kind LIKE 'operation_%' THEN 'operation-ledger'
 WHEN NEW.kind LIKE '%history_shared' OR NEW.kind='history_shared' THEN 'history-share'
 WHEN NEW.kind LIKE 'voice_%' THEN 'provider-observation'
 ELSE 'connect-journal' END,
 'correlation_id',COALESCE(json_extract(NEW.data_json,'$.receipt.delivery_id'),
 CASE WHEN NEW.kind LIKE 'operation_%' THEN json_extract(NEW.data_json,'$.operation_id') END,
 NEW.message_id,json_extract(NEW.session_json,'$.id'),json_extract(NEW.artifact_json,'$.id'),json_extract(NEW.call_json,'$.id'),json_extract(NEW.data_json,'$.source_id'),NEW.conversation_id)))
 ,data_json=CASE WHEN json_extract(NEW.data_json,'$.operation_id') IS NOT NULL THEN NEW.data_json
 WHEN COALESCE((SELECT operation_id FROM operations WHERE record_id=NEW.operation_record_id AND owner_id=NEW.recipient),(SELECT operation_id FROM journal_context WHERE singleton=1 AND actor=NEW.recipient)) IS NOT NULL
 THEN json_set(COALESCE(NEW.data_json,'{}'),'$.operation_id',COALESCE((SELECT operation_id FROM operations WHERE record_id=NEW.operation_record_id AND owner_id=NEW.recipient),(SELECT operation_id FROM journal_context WHERE singleton=1 AND actor=NEW.recipient))) ELSE NEW.data_json END
 WHERE sequence=NEW.sequence;
END;
CREATE TRIGGER journal_fact_immutable BEFORE UPDATE ON journal
 WHEN OLD.source_json IS NOT NULL AND (NEW.resource_revision IS NOT OLD.resource_revision OR NEW.source_json IS NOT OLD.source_json OR NEW.kind!=OLD.kind OR NEW.message_id IS NOT OLD.message_id OR NEW.session_json IS NOT OLD.session_json OR NEW.artifact_json IS NOT OLD.artifact_json OR NEW.call_json IS NOT OLD.call_json OR NEW.recipient!=OLD.recipient OR NEW.conversation_id!=OLD.conversation_id OR
 (OLD.observed_at>0 AND NEW.observed_at!=OLD.observed_at) OR
 (OLD.recipient_participant_id IS NOT NULL AND NEW.recipient_participant_id IS NOT OLD.recipient_participant_id) OR
 (OLD.recipient_epoch IS NOT NULL AND NEW.recipient_epoch IS NOT OLD.recipient_epoch) OR
 (NEW.data_json IS NOT OLD.data_json AND json_remove(NEW.data_json,'$.decision_record_id') IS NOT json_remove(OLD.data_json,'$.decision_record_id')))
 BEGIN SELECT RAISE(ABORT,'journal fact metadata is immutable'); END;
CREATE TRIGGER journal_fact_required AFTER UPDATE OF source_json ON journal
 WHEN NEW.source_json IS NOT NULL AND (NEW.resource_revision IS NULL OR json_type(NEW.source_json,'$.kind') IS NOT 'text' OR json_type(NEW.source_json,'$.correlation_id') IS NOT 'text')
 BEGIN SELECT RAISE(ABORT,'journal fact requires original revision and provenance'); END;
PRAGMA user_version=27;
