-- A public assistant provider instance belongs to one exact native incarnation.
-- A stable logical request reserves one fresh provider creation and export.
CREATE TABLE native_assistant_providers (
 delivery_id TEXT PRIMARY KEY REFERENCES context_deliveries(id),
 owner_id TEXT NOT NULL REFERENCES identities(id),
 operation_id TEXT NOT NULL CHECK(length(operation_id) BETWEEN 1 AND 128),
 context_credential_id TEXT NOT NULL REFERENCES credentials(id),
 connection_id TEXT NOT NULL REFERENCES connections(id),
 generation INTEGER NOT NULL CHECK(generation>0),
 runtime_epoch TEXT NOT NULL,
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 endpoint_json TEXT NOT NULL CHECK(json_valid(endpoint_json)),
 audience_json TEXT NOT NULL CHECK(json_valid(audience_json)),
 attempt_id TEXT NOT NULL UNIQUE,
 prepared_expires_at INTEGER NOT NULL,
 creation_started INTEGER NOT NULL DEFAULT 0 CHECK(creation_started IN (0,1)),
 creation_expires_at INTEGER,
 provider_ref TEXT UNIQUE CHECK(provider_ref IS NULL OR length(provider_ref) BETWEEN 1 AND 512),
 created_at INTEGER,
 export_started INTEGER NOT NULL DEFAULT 0 CHECK(export_started IN (0,1)),
 export_expires_at INTEGER,
 delivered_at INTEGER,
 rejected_at INTEGER,
 problem TEXT CHECK(problem IS NULL OR problem IN ('rejected','truncated','incomplete','unknown')),
 end_started INTEGER NOT NULL DEFAULT 0 CHECK(end_started IN (0,1)),
 end_expires_at INTEGER,
 end_uncertain INTEGER NOT NULL DEFAULT 0 CHECK(end_uncertain IN (0,1)),
 uncertain INTEGER NOT NULL DEFAULT 0 CHECK(uncertain IN (0,1)),
 closed_at INTEGER,
 UNIQUE(owner_id,operation_id),
 CHECK((creation_expires_at IS NULL)=(creation_started=0)),
 CHECK((export_expires_at IS NULL)=(export_started=0)),
 CHECK((provider_ref IS NULL)=(created_at IS NULL)),
 CHECK(created_at IS NULL OR creation_started=1),
 CHECK(export_started=0 OR created_at IS NOT NULL),
 CHECK(delivered_at IS NULL OR export_started=1),
 CHECK((end_expires_at IS NULL)=(end_started=0)),
 CHECK(closed_at IS NULL OR created_at IS NOT NULL)
);
CREATE TRIGGER native_assistant_provider_binding BEFORE INSERT ON native_assistant_providers
 WHEN NOT EXISTS(SELECT 1 FROM connections c JOIN sessions s ON s.id=c.session_id JOIN context_deliveries d ON d.id=NEW.delivery_id
 JOIN credentials k ON k.id=NEW.context_credential_id
 JOIN endpoints e ON e.id=NEW.endpoint_id AND e.identity_id=c.identity_id
 WHERE c.id=NEW.connection_id AND c.generation=NEW.generation
 AND d.conversation_id=s.conversation_id AND d.session_id=c.session_id
 AND d.owner_id=NEW.owner_id AND NEW.owner_id=k.identity_id AND d.provider='vapi' AND d.state='pending'
 AND e.transport='vapi-voice' AND e.verification_state='verified')
 BEGIN SELECT RAISE(ABORT,'assistant provider requires original native binding'); END;
CREATE TRIGGER native_assistant_provider_immutable BEFORE UPDATE OF delivery_id,owner_id,operation_id,context_credential_id,connection_id,generation,runtime_epoch,endpoint_id,endpoint_json,audience_json,attempt_id,prepared_expires_at ON native_assistant_providers
 BEGIN SELECT RAISE(ABORT,'assistant provider binding is immutable'); END;
CREATE TRIGGER native_assistant_provider_retained BEFORE DELETE ON native_assistant_providers
 BEGIN SELECT RAISE(ABORT,'assistant provider effect identity remains reserved'); END;
CREATE TRIGGER native_assistant_delivery_selection BEFORE UPDATE OF selection_json,watermark,owner_id,session_id,conversation_id,provider ON context_deliveries
 WHEN EXISTS(SELECT 1 FROM native_assistant_providers p WHERE p.delivery_id=OLD.id)
 BEGIN SELECT RAISE(ABORT,'assistant provider selection is immutable'); END;
CREATE TRIGGER native_assistant_delivery_custody BEFORE UPDATE OF state,provider_ref ON context_deliveries
 WHEN EXISTS(SELECT 1 FROM native_assistant_providers p WHERE p.delivery_id=OLD.id)
 AND (NEW.provider_ref IS NOT NULL AND NOT EXISTS(SELECT 1 FROM native_assistant_providers p WHERE p.delivery_id=OLD.id AND p.provider_ref=NEW.provider_ref)
 OR NEW.state='delivered' AND NOT EXISTS(SELECT 1 FROM native_assistant_providers p WHERE p.delivery_id=OLD.id AND p.delivered_at IS NOT NULL AND p.provider_ref=NEW.provider_ref)
 OR NEW.state='uncertain' AND NOT EXISTS(SELECT 1 FROM native_assistant_providers p WHERE p.delivery_id=OLD.id AND p.uncertain=1)
 OR NEW.state='failed' AND NOT EXISTS(SELECT 1 FROM native_assistant_providers p WHERE p.delivery_id=OLD.id AND p.rejected_at IS NOT NULL))
 BEGIN SELECT RAISE(ABORT,'assistant delivery requires captured observation'); END;
PRAGMA user_version=62;
