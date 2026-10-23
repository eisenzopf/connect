-- Provider-controlled telephone calls are effects of an ordinary Session invite.
-- They are not local-media Connections and can never manufacture ConnectionReady.
CREATE TABLE provider_session_voice (
 id TEXT PRIMARY KEY,
 invitation_record_id TEXT NOT NULL REFERENCES operation_control_receipts(operation_record_id),
 caller_credential_id TEXT NOT NULL REFERENCES credentials(id),
 target_participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 source_json TEXT NOT NULL CHECK(json_valid(source_json) AND length(source_json)<=65536),
 policy_json TEXT NOT NULL CHECK(json_valid(policy_json) AND length(policy_json)<=8192),
 projection_json TEXT NOT NULL CHECK(json_valid(projection_json) AND length(projection_json)<=1048576),
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision BETWEEN 1 AND 9007199254740991),
 expires_at INTEGER NOT NULL,
 create_started INTEGER NOT NULL DEFAULT 0 CHECK(create_started IN (0,1)),
 create_effect_id TEXT UNIQUE REFERENCES operation_effects(id),
 end_started INTEGER NOT NULL DEFAULT 0 CHECK(end_started IN (0,1)),
 end_effect_id TEXT UNIQUE REFERENCES operation_effects(id),
 attempt_runtime TEXT,
 end_runtime TEXT,
 provider_id TEXT,
 control_url TEXT CHECK(control_url IS NULL OR length(control_url)<=4096),
 ended_reason TEXT CHECK(ended_reason IS NULL OR length(ended_reason)<=256),
 quarantine_json TEXT CHECK(quarantine_json IS NULL OR (json_valid(quarantine_json) AND length(quarantine_json)<=16384)),
 quarantined_at INTEGER,
 observed_at INTEGER,
 closed_at INTEGER,
 CHECK((quarantine_json IS NULL)=(quarantined_at IS NULL)),
 CHECK((create_started=0 AND create_effect_id IS NULL AND attempt_runtime IS NULL)
 OR (create_started=1 AND create_effect_id IS NOT NULL AND attempt_runtime IS NOT NULL)),
 CHECK((end_started=0 AND end_effect_id IS NULL AND end_runtime IS NULL)
 OR (end_started=1 AND end_effect_id IS NOT NULL AND end_runtime IS NOT NULL AND provider_id IS NOT NULL AND create_started=1)),
 UNIQUE(invitation_record_id,target_participant_id)
);
CREATE UNIQUE INDEX provider_voice_resource_once ON provider_session_voice(
 json_extract(policy_json,'$.provider'),json_extract(policy_json,'$.route_reference'),
 json_extract(source_json,'$.route_owner_id'),provider_id) WHERE provider_id IS NOT NULL;
CREATE TRIGGER provider_voice_source_binding BEFORE INSERT ON provider_session_voice
 WHEN NOT EXISTS(SELECT 1 FROM operations o JOIN credentials k ON k.id=NEW.caller_credential_id AND k.identity_id=o.owner_id
 JOIN endpoints e ON e.id=NEW.endpoint_id JOIN participants p ON p.participant_id=NEW.target_participant_id AND p.identity_id=e.identity_id
 WHERE o.record_id=NEW.invitation_record_id AND o.command_type='session.invite' AND o.state='completed'
 AND o.conversation_id=p.conversation_id AND o.session_id=json_extract(NEW.source_json,'$.request.session_id')
 AND o.record_id=json_extract(NEW.source_json,'$.invitation_record_id')
 AND NEW.target_participant_id=json_extract(NEW.source_json,'$.target.participant_id')
 AND NEW.endpoint_id=json_extract(NEW.source_json,'$.endpoint_id') AND NEW.create_started=0 AND NEW.end_started=0)
 BEGIN SELECT RAISE(ABORT,'provider voice Session binding mismatch'); END;
CREATE TRIGGER provider_voice_binding_immutable BEFORE UPDATE ON provider_session_voice
 WHEN NEW.id!=OLD.id OR NEW.invitation_record_id!=OLD.invitation_record_id
 OR NEW.caller_credential_id!=OLD.caller_credential_id OR NEW.target_participant_id!=OLD.target_participant_id
 OR NEW.endpoint_id!=OLD.endpoint_id OR NEW.source_json!=OLD.source_json OR NEW.policy_json!=OLD.policy_json
 OR NEW.revision<OLD.revision
 OR NEW.expires_at!=OLD.expires_at OR NEW.create_started<OLD.create_started OR NEW.end_started<OLD.end_started
 OR (OLD.create_effect_id IS NOT NULL AND NEW.create_effect_id IS NOT OLD.create_effect_id)
 OR (OLD.end_effect_id IS NOT NULL AND NEW.end_effect_id IS NOT OLD.end_effect_id)
 OR (OLD.attempt_runtime IS NOT NULL AND NEW.attempt_runtime IS NOT OLD.attempt_runtime)
 OR (OLD.end_runtime IS NOT NULL AND NEW.end_runtime IS NOT OLD.end_runtime)
 OR (OLD.provider_id IS NOT NULL AND NEW.provider_id IS NOT OLD.provider_id)
 OR (OLD.quarantine_json IS NOT NULL AND NEW.quarantine_json IS NOT OLD.quarantine_json)
 OR (OLD.quarantined_at IS NOT NULL AND NEW.quarantined_at IS NOT OLD.quarantined_at)
 OR (OLD.closed_at IS NOT NULL AND NEW.closed_at IS NOT OLD.closed_at)
 BEGIN SELECT RAISE(ABORT,'provider voice binding is immutable'); END;
CREATE TRIGGER provider_voice_create_effect_binding BEFORE UPDATE OF create_effect_id ON provider_session_voice
 WHEN NEW.create_effect_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM operation_effects e
 WHERE e.id=NEW.create_effect_id AND e.operation_record_id=NEW.invitation_record_id
 AND e.correlation_key=NEW.id AND e.compensates_effect_id IS NULL AND e.attempt=1)
 BEGIN SELECT RAISE(ABORT,'provider voice create effect mismatch'); END;
CREATE TRIGGER provider_voice_end_effect_binding BEFORE UPDATE OF end_effect_id ON provider_session_voice
 WHEN NEW.end_effect_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM operation_effects e
 WHERE e.id=NEW.end_effect_id AND e.operation_record_id=NEW.invitation_record_id
 AND e.correlation_key='cleanup_'||NEW.id AND e.compensates_effect_id=NEW.create_effect_id AND e.attempt=1)
 BEGIN SELECT RAISE(ABORT,'provider voice cleanup effect mismatch'); END;
CREATE TRIGGER provider_voice_retained BEFORE DELETE ON provider_session_voice
 BEGIN SELECT RAISE(ABORT,'provider voice identity remains reserved'); END;
CREATE TRIGGER provider_voice_no_second_host BEFORE INSERT ON provider_session_voice
 WHEN EXISTS(SELECT 1 FROM initial_telephone_operations i WHERE i.invitation_record_id=NEW.invitation_record_id AND i.target_participant_id=NEW.target_participant_id)
 BEGIN SELECT RAISE(ABORT,'Session target already has a telephone host'); END;
CREATE TRIGGER initial_telephone_no_second_host BEFORE INSERT ON initial_telephone_operations
 WHEN EXISTS(SELECT 1 FROM provider_session_voice p WHERE p.invitation_record_id=NEW.invitation_record_id AND p.target_participant_id=NEW.target_participant_id)
 BEGIN SELECT RAISE(ABORT,'Session target already has a provider host'); END;
PRAGMA user_version=67;
