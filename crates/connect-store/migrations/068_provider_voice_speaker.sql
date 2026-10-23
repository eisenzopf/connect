ALTER TABLE provider_session_voice ADD COLUMN refusal_code TEXT CHECK(refusal_code IS NULL OR length(refusal_code) BETWEEN 1 AND 128);
-- An actual configured assistant must be the owning AI Participant, rather than
-- an implicit speaker represented only by a provider assistant string.
CREATE INDEX provider_voice_conversation ON provider_session_voice(json_extract(source_json,'$.request.conversation_id'),id);
CREATE TRIGGER provider_voice_assistant_binding BEFORE INSERT ON provider_session_voice
 WHEN NOT EXISTS(SELECT 1 FROM endpoints e JOIN identities i ON i.id=e.identity_id AND i.kind='ai' AND i.revoked=0
 JOIN devices d ON d.id=e.device_id AND d.identity_id=i.id AND d.revoked=0
 JOIN credentials k ON k.id=json_extract(NEW.policy_json,'$.assistant_credential_id') AND k.identity_id=i.id
 JOIN operations parent ON parent.record_id=NEW.invitation_record_id AND parent.owner_id=i.id
 WHERE e.id=json_extract(NEW.policy_json,'$.assistant_endpoint_id') AND e.transport='vapi-voice'
 AND e.verification_state='verified' AND e.address=json_extract(NEW.policy_json,'$.assistant_id')
 AND e.secret_ref=json_extract(NEW.policy_json,'$.route_reference')
 AND e.route_owner_id=json_extract(NEW.source_json,'$.route_owner_id')
 AND e.revision=json_extract(NEW.source_json,'$.assistant_endpoint_revision')
 AND e.device_id=json_extract(NEW.source_json,'$.assistant_device_id'))
 BEGIN SELECT RAISE(ABORT,'provider assistant is not the owning AI Participant'); END;
PRAGMA user_version=68;
