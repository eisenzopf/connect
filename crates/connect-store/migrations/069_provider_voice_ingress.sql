-- Incoming observations never submit an outbound creation and never accept a
-- Participant. The intended audience is explicit configuration, not Caller ID.
CREATE TABLE provider_voice_ingress (
 id TEXT PRIMARY KEY,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 policy_json TEXT NOT NULL CHECK(json_valid(policy_json) AND length(policy_json)<=8192),
 source_json TEXT NOT NULL CHECK(json_valid(source_json) AND length(source_json)<=16384),
 worker_id TEXT NOT NULL,
 since INTEGER NOT NULL CHECK(since>=0),
 scanned_until INTEGER NOT NULL CHECK(scanned_until>=since*1000)
);
ALTER TABLE provider_session_voice ADD COLUMN direction TEXT NOT NULL DEFAULT 'outbound' CHECK(direction IN ('inbound','outbound'));
ALTER TABLE provider_session_voice ADD COLUMN ingress_id TEXT REFERENCES provider_voice_ingress(id);
CREATE UNIQUE INDEX provider_voice_ingress_selection ON provider_voice_ingress(worker_id,
 json_extract(policy_json,'$.provider'),json_extract(policy_json,'$.route_reference'),
 json_extract(source_json,'$.route_owner_id'),json_extract(policy_json,'$.assistant_id'),
 json_extract(policy_json,'$.phone_number_id'),json_extract(source_json,'$.endpoint.address'));
CREATE TRIGGER provider_voice_ingress_immutable BEFORE UPDATE ON provider_voice_ingress
 WHEN NEW.id!=OLD.id OR NEW.conversation_id!=OLD.conversation_id OR NEW.policy_json!=OLD.policy_json
 OR NEW.source_json!=OLD.source_json OR NEW.since!=OLD.since OR NEW.scanned_until<OLD.scanned_until
 BEGIN SELECT RAISE(ABORT,'provider ingress binding remains immutable'); END;
CREATE TRIGGER provider_voice_ingress_retained BEFORE DELETE ON provider_voice_ingress
 BEGIN SELECT RAISE(ABORT,'provider ingress identities and cursors remain reserved'); END;
CREATE TRIGGER provider_voice_direction_immutable BEFORE UPDATE OF direction,ingress_id ON provider_session_voice
 WHEN NEW.direction!=OLD.direction OR NEW.ingress_id IS NOT OLD.ingress_id
 BEGIN SELECT RAISE(ABORT,'provider call direction remains immutable'); END;
CREATE TRIGGER provider_voice_inbound_binding BEFORE INSERT ON provider_session_voice
 WHEN (NEW.direction='inbound')!=(NEW.ingress_id IS NOT NULL)
 OR (NEW.ingress_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM provider_voice_ingress i
 WHERE i.id=NEW.ingress_id AND i.policy_json=NEW.policy_json AND i.conversation_id=json_extract(NEW.source_json,'$.request.conversation_id')))
 BEGIN SELECT RAISE(ABORT,'provider incoming call ingress mismatch'); END;
PRAGMA user_version=69;
