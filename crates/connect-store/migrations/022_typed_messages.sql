-- Fresh schema boundary for explicit incoming policy and typed message metadata.
ALTER TABLE connector_routes ADD COLUMN incoming_to_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(incoming_to_json));
ALTER TABLE provider_jobs ADD COLUMN refusal_code TEXT;
CREATE TRIGGER explicit_message_metadata AFTER INSERT ON messages WHEN NEW.audience_mode='explicit' BEGIN
 UPDATE messages SET sender_participant_id=COALESCE(NEW.sender_participant_id,(SELECT participant_id FROM participants WHERE conversation_id=NEW.conversation_id AND identity_id=NEW.sender AND active=1)),accepted_at=COALESCE(NEW.accepted_at,strftime('%s','now')) WHERE id=NEW.id;
END;
PRAGMA user_version=22;
