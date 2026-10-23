-- A provider memory belongs to one private physical context, never a Session
-- or logical AI identity alone. Call/export custody is retained across failure.
CREATE TABLE private_provider_contexts (
 delivery_id TEXT PRIMARY KEY REFERENCES context_deliveries(id),
 operation_record_id TEXT NOT NULL UNIQUE REFERENCES operations(record_id),
 connection_id TEXT NOT NULL UNIQUE REFERENCES connections(id),
 runtime_epoch TEXT NOT NULL,
 source_digest TEXT NOT NULL,
 selection_json TEXT NOT NULL CHECK(json_valid(selection_json) AND length(selection_json)<=32768),
 audience_json TEXT NOT NULL CHECK(json_valid(audience_json)),
 context_credential_id TEXT NOT NULL REFERENCES credentials(id),
 delegate_credential_id TEXT NOT NULL REFERENCES credentials(id),
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 endpoint_json TEXT NOT NULL CHECK(json_valid(endpoint_json)),
 attempt_id TEXT NOT NULL UNIQUE,
 started INTEGER NOT NULL DEFAULT 0 CHECK(started IN (0,1)),
 creation_expires_at INTEGER,
 uncertain INTEGER NOT NULL DEFAULT 0 CHECK(uncertain IN (0,1)),
 provider_ref TEXT UNIQUE CHECK(provider_ref IS NULL OR length(provider_ref) BETWEEN 1 AND 512),
 created_at INTEGER,
 export_started INTEGER NOT NULL DEFAULT 0 CHECK(export_started IN (0,1)),
 export_expires_at INTEGER,
 export_uncertain INTEGER NOT NULL DEFAULT 0 CHECK(export_uncertain IN (0,1)),
 delivered_at INTEGER,
 closed_at INTEGER,
 CHECK((creation_expires_at IS NULL)=(started=0)),
 CHECK((export_expires_at IS NULL)=(export_started=0)),
 CHECK(closed_at IS NULL OR created_at IS NOT NULL),
 CHECK((created_at IS NULL)=(provider_ref IS NULL)),
 CHECK(created_at IS NULL OR started=1),
 CHECK(delivered_at IS NULL OR (started=1 AND export_started=1 AND provider_ref IS NOT NULL))
);
CREATE TRIGGER private_provider_binding BEFORE INSERT ON private_provider_contexts
 WHEN NOT EXISTS(SELECT 1 FROM native_consultation_legs l JOIN operations o ON o.record_id=l.operation_record_id
 JOIN context_deliveries d ON d.id=NEW.delivery_id JOIN connections v ON v.id=l.connection_id
 JOIN endpoints ep ON ep.id=NEW.endpoint_id AND ep.identity_id=v.identity_id AND ep.transport='vapi-voice' AND ep.verification_state='verified'
 WHERE l.operation_record_id=NEW.operation_record_id AND l.connection_id=NEW.connection_id
 AND l.runtime_epoch=NEW.runtime_epoch AND l.original_digest=NEW.source_digest AND l.audience_json=NEW.audience_json
 AND o.command_type='session.consult' AND o.state IN ('accepted','preparing')
 AND d.session_id=o.session_id AND d.conversation_id=o.conversation_id AND d.provider='vapi' AND d.state='pending'
 AND NEW.started=0 AND NEW.uncertain=0 AND NEW.provider_ref IS NULL AND NEW.delivered_at IS NULL
 AND NEW.export_started=0 AND NEW.export_uncertain=0 AND NEW.created_at IS NULL AND NEW.closed_at IS NULL)
 BEGIN SELECT RAISE(ABORT,'private provider source binding mismatch'); END;
CREATE TRIGGER private_provider_immutable BEFORE UPDATE ON private_provider_contexts
 WHEN NEW.delivery_id!=OLD.delivery_id OR NEW.operation_record_id!=OLD.operation_record_id OR NEW.connection_id!=OLD.connection_id
 OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.source_digest!=OLD.source_digest OR NEW.selection_json!=OLD.selection_json
 OR NEW.audience_json!=OLD.audience_json OR NEW.context_credential_id!=OLD.context_credential_id OR NEW.delegate_credential_id!=OLD.delegate_credential_id
 OR NEW.endpoint_id!=OLD.endpoint_id OR NEW.endpoint_json!=OLD.endpoint_json OR NEW.attempt_id!=OLD.attempt_id
 OR NEW.started<OLD.started OR NEW.uncertain<OLD.uncertain OR NEW.export_started<OLD.export_started OR NEW.export_uncertain<OLD.export_uncertain
 OR (OLD.created_at IS NOT NULL AND NEW.created_at IS NOT OLD.created_at)
 OR (OLD.creation_expires_at IS NOT NULL AND NEW.creation_expires_at IS NOT OLD.creation_expires_at)
 OR (OLD.export_expires_at IS NOT NULL AND NEW.export_expires_at IS NOT OLD.export_expires_at)
 OR (OLD.provider_ref IS NOT NULL AND NEW.provider_ref IS NOT OLD.provider_ref)
 OR (OLD.delivered_at IS NOT NULL AND NEW.delivered_at IS NOT OLD.delivered_at)
 OR (OLD.closed_at IS NOT NULL AND NEW.closed_at IS NOT OLD.closed_at)
 BEGIN SELECT RAISE(ABORT,'private provider context is immutable'); END;
CREATE TRIGGER private_provider_retained BEFORE DELETE ON private_provider_contexts
 BEGIN SELECT RAISE(ABORT,'private provider context remains reserved'); END;
-- Generic context outcome APIs cannot complete a private initialized memory.
-- The sealed runtime recorder first records the matching captured observation.
CREATE TRIGGER private_provider_delivery_outcome BEFORE UPDATE OF state,provider_ref ON context_deliveries
 WHEN EXISTS(SELECT 1 FROM private_provider_contexts p WHERE p.delivery_id=OLD.id)
 AND NOT((NEW.state='delivered' AND EXISTS(SELECT 1 FROM private_provider_contexts p WHERE p.delivery_id=OLD.id AND p.delivered_at IS NOT NULL AND p.provider_ref=NEW.provider_ref)) OR (NEW.state='uncertain' AND NEW.provider_ref IS NULL AND EXISTS(SELECT 1 FROM private_provider_contexts p WHERE p.delivery_id=OLD.id AND (p.uncertain=1 OR p.export_uncertain=1) AND p.delivered_at IS NULL)))
 BEGIN SELECT RAISE(ABORT,'private context requires captured delivery custody'); END;
CREATE TRIGGER private_provider_delivery_selection BEFORE UPDATE OF selection_json,watermark,owner_id,session_id,conversation_id,provider ON context_deliveries
 WHEN EXISTS(SELECT 1 FROM private_provider_contexts p WHERE p.delivery_id=OLD.id)
 AND (NEW.selection_json!=OLD.selection_json OR NEW.watermark!=OLD.watermark OR NEW.owner_id IS NOT OLD.owner_id OR NEW.session_id IS NOT OLD.session_id OR NEW.conversation_id!=OLD.conversation_id OR NEW.provider!=OLD.provider)
 BEGIN SELECT RAISE(ABORT,'private provider selection is immutable'); END;
CREATE TABLE private_content_sources (
 kind TEXT NOT NULL CHECK(kind IN ('message','context_delivery')),
 source_id TEXT NOT NULL,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 operation_record_id TEXT NOT NULL REFERENCES private_provider_contexts(operation_record_id),
 connection_id TEXT NOT NULL REFERENCES connections(id),
 PRIMARY KEY(kind,source_id)
);
CREATE TRIGGER private_content_source_binding BEFORE INSERT ON private_content_sources
 WHEN NOT EXISTS(SELECT 1 FROM private_provider_contexts p JOIN context_deliveries d ON d.id=p.delivery_id
 WHERE p.operation_record_id=NEW.operation_record_id AND p.connection_id=NEW.connection_id AND d.conversation_id=NEW.conversation_id
 AND ((NEW.kind='context_delivery' AND NEW.source_id=p.delivery_id) OR (NEW.kind='message' AND EXISTS(SELECT 1 FROM messages m WHERE m.id=NEW.source_id AND m.conversation_id=d.conversation_id))))
 BEGIN SELECT RAISE(ABORT,'private content requires physical context provenance'); END;
CREATE TRIGGER private_content_source_immutable BEFORE UPDATE ON private_content_sources BEGIN SELECT RAISE(ABORT,'private content provenance is immutable'); END;
CREATE TRIGGER private_content_source_retained BEFORE DELETE ON private_content_sources BEGIN SELECT RAISE(ABORT,'private content provenance remains reserved'); END;
PRAGMA user_version=52;
