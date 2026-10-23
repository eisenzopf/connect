-- A successful credential rotation does not rewrite the original offer/effect.
-- Only an observed Core credential commit selects a successor media authority.
CREATE TABLE native_media_credential_bindings (
 id TEXT PRIMARY KEY CHECK(length(id) BETWEEN 1 AND 128),
 parent_record_id TEXT NOT NULL REFERENCES native_connection_lifetimes(operation_record_id),
 refresh_id TEXT NOT NULL CHECK(length(refresh_id) BETWEEN 1 AND 128),
 runtime_epoch TEXT NOT NULL,
 epoch INTEGER NOT NULL CHECK(epoch BETWEEN 1 AND 9007199254740990),
 previous_credential_id TEXT NOT NULL REFERENCES credentials(id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 key_id TEXT NOT NULL REFERENCES peer_signing_keys(key_id),
 device_id TEXT NOT NULL REFERENCES devices(id),
 source_expires_at INTEGER NOT NULL,
 stream_revision INTEGER NOT NULL CHECK(stream_revision BETWEEN 1 AND 9007199254740991),
 snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json) AND length(CAST(snapshot_json AS BLOB))<=32768),
 started INTEGER NOT NULL DEFAULT 0 CHECK(started IN (0,1)),
 state TEXT NOT NULL DEFAULT 'prepared' CHECK(state IN ('prepared','applied','refused','uncertain')),
 core_credential_revision INTEGER,
 created_at INTEGER NOT NULL,
 observed_at INTEGER,
 UNIQUE(parent_record_id,refresh_id),
 CHECK((state='applied' AND started=1 AND core_credential_revision=epoch+1 AND observed_at IS NOT NULL)
    OR (state!='applied' AND core_credential_revision IS NULL))
);
CREATE INDEX native_media_credential_current ON native_media_credential_bindings(parent_record_id,runtime_epoch,state,epoch DESC);
CREATE UNIQUE INDEX native_media_credential_epoch_reserved
 ON native_media_credential_bindings(parent_record_id,epoch) WHERE state!='refused';
CREATE TRIGGER native_media_credential_binding_insert BEFORE INSERT ON native_media_credential_bindings
 WHEN NEW.started!=0 OR NEW.state!='prepared' OR NEW.observed_at IS NOT NULL OR NOT EXISTS(
 SELECT 1 FROM native_connection_lifetimes life
 JOIN connections c ON c.id=life.connection_id
 JOIN native_original_admission_sources n ON n.operation_record_id=life.operation_record_id
 JOIN credentials old ON old.id=NEW.previous_credential_id
 JOIN credentials next ON next.id=NEW.credential_id AND next.identity_id=old.identity_id
 JOIN peer_signing_keys key ON key.key_id=NEW.key_id AND key.identity_id=next.identity_id AND key.device_id=NEW.device_id
 JOIN credential_signing_keys first ON first.credential_id=old.id AND first.key_id=key.key_id
 JOIN credential_signing_keys last ON last.credential_id=next.id AND last.key_id=key.key_id
 WHERE life.operation_record_id=NEW.parent_record_id AND life.runtime_epoch=NEW.runtime_epoch AND life.retired=0
 AND c.generation=life.generation AND c.observed_runtime_id=life.runtime_epoch AND c.identity_id=next.identity_id
 AND n.state='completed' AND n.attempt_runtime=NEW.runtime_epoch AND n.key_id=NEW.key_id AND n.device_id=NEW.device_id
 AND NEW.previous_credential_id=COALESCE((SELECT credential_id FROM native_media_credential_bindings prior WHERE prior.parent_record_id=NEW.parent_record_id AND prior.state='applied' ORDER BY epoch DESC LIMIT 1),n.credential_id)
 AND c.state IN ('ready','active') AND json_extract(c.readiness_json,'$.stream_revision')=NEW.stream_revision
 AND old.revoked=0 AND old.expires_at>NEW.created_at AND next.revoked=0 AND next.expires_at>=NEW.source_expires_at
 AND key.revoked=0 AND key.not_before<=NEW.created_at AND key.expires_at>=NEW.source_expires_at
 AND NEW.source_expires_at>NEW.created_at
 AND NEW.epoch=COALESCE((SELECT max(epoch) FROM native_media_credential_bindings b WHERE b.parent_record_id=NEW.parent_record_id AND b.state='applied'),0)+1)
 BEGIN SELECT RAISE(ABORT,'native credential rotation binding mismatch'); END;
CREATE TRIGGER native_media_credential_binding_immutable BEFORE UPDATE ON native_media_credential_bindings
 WHEN NEW.id!=OLD.id OR NEW.parent_record_id!=OLD.parent_record_id OR NEW.refresh_id!=OLD.refresh_id
 OR NEW.runtime_epoch!=OLD.runtime_epoch OR NEW.epoch!=OLD.epoch
 OR NEW.previous_credential_id!=OLD.previous_credential_id OR NEW.credential_id!=OLD.credential_id
 OR NEW.key_id!=OLD.key_id OR NEW.device_id!=OLD.device_id OR NEW.source_expires_at!=OLD.source_expires_at
 OR NEW.stream_revision!=OLD.stream_revision OR NEW.snapshot_json!=OLD.snapshot_json OR NEW.created_at!=OLD.created_at
 OR (OLD.state!='prepared' AND NOT (OLD.state='uncertain' AND OLD.started=1 AND NEW.state='applied')) OR NEW.started<OLD.started
 OR NOT ((OLD.started=0 AND NEW.started=1 AND NEW.state='prepared' AND NEW.observed_at IS NULL)
      OR (NEW.state='refused' AND OLD.started=0 AND NEW.observed_at IS NOT NULL)
      OR (NEW.state='uncertain' AND OLD.started=1 AND NEW.observed_at IS NOT NULL)
      OR (OLD.started=1 AND NEW.started=1 AND NEW.state='applied' AND NEW.core_credential_revision=OLD.epoch+1 AND NEW.observed_at IS NOT NULL))
 BEGIN SELECT RAISE(ABORT,'native credential rotation observation is immutable'); END;
CREATE TRIGGER native_media_credential_binding_retained BEFORE DELETE ON native_media_credential_bindings
 BEGIN SELECT RAISE(ABORT,'native credential rotation remains reserved'); END;

CREATE VIEW native_original_admission_sources AS
 SELECT n.operation_record_id,n.preparation_record_id,n.credential_id,n.key_id,n.device_id,n.endpoint_id,n.participant_id,n.membership_epoch,n.generation,n.resources_json,n.source_expires_at,n.terminal_reserve,n.attempt_id,n.attempt_runtime,n.effect_id,n.effect_started,o.record_id,o.operation_id,o.owner_id,o.tenant_id,o.conversation_id,o.session_id,o.connection_id,o.expected_revision,o.state
 FROM native_offer_operations n JOIN operations o ON o.record_id=n.operation_record_id
 UNION ALL
 SELECT n.operation_record_id,NULL,n.credential_id,n.key_id,n.device_id,n.endpoint_id,n.participant_id,n.membership_epoch,n.generation,n.resources_json,n.source_expires_at,json_array_length(n.resources_json)+1,n.attempt_id,n.attempt_runtime,n.effect_id,1,
 o.record_id,o.operation_id,c.identity_id,c.tenant_id,o.conversation_id,o.session_id,c.id,0,o.state
 FROM native_replacement_lifetimes n JOIN operations o ON o.record_id=n.operation_record_id
 JOIN native_replacements r ON r.operation_record_id=n.operation_record_id JOIN connections c ON c.id=r.candidate_connection_id;
DROP VIEW native_admission_sources;
CREATE VIEW native_admission_sources AS
 SELECT n.operation_record_id,n.preparation_record_id,COALESCE(b.credential_id,n.credential_id) credential_id,
 COALESCE(b.key_id,n.key_id) key_id,COALESCE(b.device_id,n.device_id) device_id,
 n.endpoint_id,n.participant_id,n.membership_epoch,n.generation,n.resources_json,
 COALESCE(b.source_expires_at,n.source_expires_at) source_expires_at,
 n.terminal_reserve,n.attempt_id,n.attempt_runtime,n.effect_id,n.effect_started,n.record_id,n.operation_id,
 n.owner_id,n.tenant_id,n.conversation_id,n.session_id,n.connection_id,n.expected_revision,n.state
 FROM native_original_admission_sources n LEFT JOIN native_media_credential_bindings b
 ON b.parent_record_id=n.operation_record_id AND b.runtime_epoch=n.attempt_runtime AND b.state='applied'
 AND b.epoch=(SELECT max(epoch) FROM native_media_credential_bindings latest WHERE latest.parent_record_id=n.operation_record_id AND latest.runtime_epoch=n.attempt_runtime AND latest.state='applied');
PRAGMA user_version=86;
