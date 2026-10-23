-- Initialization exports history in the once-only create request, before its
-- actual provider reference is known. This admission remains visible on loss.
CREATE TABLE native_assistant_initializations (
 delivery_id TEXT PRIMARY KEY REFERENCES native_assistant_providers(delivery_id),
 started INTEGER NOT NULL DEFAULT 0 CHECK(started IN (0,1)),
 expires_at INTEGER,
 CHECK((expires_at IS NULL)=(started=0))
);
CREATE TRIGGER native_assistant_initialization_binding BEFORE INSERT ON native_assistant_initializations
 WHEN NOT EXISTS(SELECT 1 FROM native_assistant_providers p WHERE p.delivery_id=NEW.delivery_id AND p.creation_started=0)
 BEGIN SELECT RAISE(ABORT,'initialization precedes provider creation'); END;
CREATE TRIGGER native_assistant_initialization_retained BEFORE DELETE ON native_assistant_initializations
 BEGIN SELECT RAISE(ABORT,'initialization intent remains reserved'); END;
CREATE TRIGGER native_assistant_initialization_identity BEFORE UPDATE OF delivery_id ON native_assistant_initializations
 BEGIN SELECT RAISE(ABORT,'initialization binding is immutable'); END;

-- Each update has a frozen selection and its own stable logical operation.
-- Cleanup always belongs to the original captured instance, never this row.
CREATE TABLE native_assistant_context_updates (
 delivery_id TEXT PRIMARY KEY REFERENCES context_deliveries(id),
 instance_delivery_id TEXT NOT NULL REFERENCES native_assistant_providers(delivery_id),
 owner_id TEXT NOT NULL REFERENCES identities(id),
 operation_id TEXT NOT NULL CHECK(length(operation_id) BETWEEN 1 AND 128),
 attempt_id TEXT NOT NULL UNIQUE,
 ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 1 AND 16),
 prepared_expires_at INTEGER NOT NULL,
 export_started INTEGER NOT NULL DEFAULT 0 CHECK(export_started IN (0,1)),
 export_expires_at INTEGER,
 delivered_at INTEGER,
 rejected_at INTEGER,
 uncertain INTEGER NOT NULL DEFAULT 0 CHECK(uncertain IN (0,1)),
 problem TEXT CHECK(problem IS NULL OR problem IN ('rejected','truncated','incomplete','unknown')),
 UNIQUE(owner_id,operation_id),
 UNIQUE(instance_delivery_id,ordinal),
 CHECK((export_expires_at IS NULL)=(export_started=0)),
 CHECK(delivered_at IS NULL OR export_started=1)
);
CREATE TRIGGER native_assistant_update_binding BEFORE INSERT ON native_assistant_context_updates
 WHEN NOT EXISTS(SELECT 1 FROM native_assistant_providers p JOIN context_deliveries original ON original.id=p.delivery_id JOIN context_deliveries d ON d.id=NEW.delivery_id
 WHERE p.delivery_id=NEW.instance_delivery_id AND p.owner_id=NEW.owner_id AND p.delivered_at IS NOT NULL
 AND p.uncertain=0 AND p.end_started=0 AND p.closed_at IS NULL
 AND d.owner_id=p.owner_id AND d.conversation_id=original.conversation_id AND d.session_id=original.session_id
 AND d.provider='vapi' AND d.provider_ref=p.provider_ref AND d.state='pending')
 BEGIN SELECT RAISE(ABORT,'context update requires its captured delivered instance'); END;
CREATE TRIGGER native_assistant_update_identity BEFORE UPDATE OF delivery_id,instance_delivery_id,owner_id,operation_id,attempt_id,ordinal,prepared_expires_at ON native_assistant_context_updates
 BEGIN SELECT RAISE(ABORT,'context update binding is immutable'); END;
CREATE TRIGGER native_assistant_update_retained BEFORE DELETE ON native_assistant_context_updates
 BEGIN SELECT RAISE(ABORT,'context update intent remains reserved'); END;
CREATE TRIGGER native_assistant_update_selection BEFORE UPDATE OF selection_json,watermark,owner_id,session_id,conversation_id,provider,provider_ref ON context_deliveries
 WHEN EXISTS(SELECT 1 FROM native_assistant_context_updates u WHERE u.delivery_id=OLD.id)
 BEGIN SELECT RAISE(ABORT,'context update selection and instance are immutable'); END;
CREATE TRIGGER native_assistant_update_custody BEFORE UPDATE OF state ON context_deliveries
 WHEN EXISTS(SELECT 1 FROM native_assistant_context_updates u WHERE u.delivery_id=OLD.id)
 AND (NEW.state='delivered' AND NOT EXISTS(SELECT 1 FROM native_assistant_context_updates u WHERE u.delivery_id=OLD.id AND u.delivered_at IS NOT NULL)
 OR NEW.state='uncertain' AND NOT EXISTS(SELECT 1 FROM native_assistant_context_updates u WHERE u.delivery_id=OLD.id AND u.uncertain=1)
 OR NEW.state='failed' AND NOT EXISTS(SELECT 1 FROM native_assistant_context_updates u WHERE u.delivery_id=OLD.id AND u.rejected_at IS NOT NULL))
 BEGIN SELECT RAISE(ABORT,'context update requires captured observation'); END;
PRAGMA user_version=63;
