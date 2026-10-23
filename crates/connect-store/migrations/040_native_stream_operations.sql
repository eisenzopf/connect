-- Each native control has its own shared logical identity and one uncertain
-- engine attempt. A live parent never substitutes for this command's grant.
CREATE TABLE native_stream_operations (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 parent_record_id TEXT NOT NULL REFERENCES native_connection_lifetimes(operation_record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 key_id TEXT NOT NULL REFERENCES peer_signing_keys(key_id),
 device_id TEXT NOT NULL REFERENCES devices(id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
 resources_json TEXT NOT NULL CHECK(json_valid(resources_json) AND json_type(resources_json)='array'),
 source_expires_at INTEGER NOT NULL,
 closed INTEGER NOT NULL DEFAULT 0 CHECK(closed IN (0,1)),
 effect_started INTEGER NOT NULL DEFAULT 0 CHECK(effect_started IN (0,1)),
 effect_id TEXT UNIQUE REFERENCES operation_effects(id),
 attempt_id TEXT,
 attempt_runtime TEXT,
 CHECK((effect_started=0 AND effect_id IS NULL AND attempt_id IS NULL AND attempt_runtime IS NULL)
    OR (effect_started=1 AND effect_id IS NOT NULL AND attempt_id IS NOT NULL AND attempt_runtime IS NOT NULL))
);
CREATE UNIQUE INDEX native_stream_single_open ON native_stream_operations(parent_record_id) WHERE closed=0;
CREATE TRIGGER native_stream_binding BEFORE INSERT ON native_stream_operations
 WHEN NOT EXISTS(SELECT 1 FROM operations o
 JOIN operations parent ON parent.record_id=NEW.parent_record_id
 JOIN native_offer_operations n ON n.operation_record_id=parent.record_id
 JOIN native_connection_lifetimes life ON life.operation_record_id=parent.record_id
 JOIN connections c ON c.id=parent.connection_id
 JOIN credentials credential ON credential.id=NEW.credential_id
 JOIN credential_signing_keys binding ON binding.credential_id=credential.id AND binding.key_id=NEW.key_id
 JOIN peer_signing_keys key ON key.key_id=binding.key_id
 WHERE o.record_id=NEW.operation_record_id AND o.ledger_version=0 AND o.state='accepted'
 AND o.command_type IN ('stream.subscribe','stream.unsubscribe')
 AND o.owner_id=parent.owner_id AND o.tenant_id=parent.tenant_id
 AND o.conversation_id=parent.conversation_id AND o.session_id=parent.session_id AND o.connection_id=parent.connection_id
 AND parent.state='completed' AND life.retired=0 AND life.runtime_epoch=n.attempt_runtime
 AND c.state IN ('ready','active') AND c.generation=NEW.generation AND c.generation=life.generation
 AND c.observed_runtime_id=life.runtime_epoch AND c.readiness_json IS NOT NULL
 AND o.expected_revision BETWEEN 0 AND 9007199254740991
 AND json_extract(o.intent_json,'$.operation_id')=o.operation_id
 AND json_extract(o.intent_json,'$.conversation_id')=o.conversation_id
 AND json_extract(o.intent_json,'$.session_id')=o.session_id
 AND json_extract(o.intent_json,'$.connection_id')=o.connection_id
 AND json_extract(o.intent_json,'$.expected_revision')=o.expected_revision
 AND json_extract(o.intent_json,'$.command')=o.command_type
 AND NEW.credential_id=n.credential_id AND NEW.key_id=n.key_id AND NEW.device_id=n.device_id
 AND NEW.participant_id=n.participant_id AND NEW.membership_epoch=n.membership_epoch
 AND NEW.resources_json=n.resources_json AND NEW.source_expires_at<=n.source_expires_at
 AND credential.identity_id=o.owner_id AND key.identity_id=o.owner_id AND key.device_id=NEW.device_id
 AND key.not_before<=o.created_at AND key.expires_at>=NEW.source_expires_at
 AND credential.expires_at>=NEW.source_expires_at AND o.expires_at<=NEW.source_expires_at
 AND NEW.closed=0 AND NEW.effect_started=0)
 BEGIN SELECT RAISE(ABORT,'native stream source binding mismatch'); END;
CREATE TRIGGER native_stream_origin_immutable BEFORE UPDATE ON native_stream_operations
 WHEN NEW.operation_record_id!=OLD.operation_record_id OR NEW.parent_record_id!=OLD.parent_record_id
 OR NEW.credential_id!=OLD.credential_id OR NEW.key_id!=OLD.key_id OR NEW.device_id!=OLD.device_id
 OR NEW.participant_id!=OLD.participant_id OR NEW.membership_epoch!=OLD.membership_epoch
 OR NEW.generation!=OLD.generation OR NEW.resources_json!=OLD.resources_json
 OR NEW.source_expires_at!=OLD.source_expires_at OR NEW.closed<OLD.closed OR NEW.effect_started<OLD.effect_started
 OR (OLD.effect_started=1 AND (NEW.effect_id IS NOT OLD.effect_id OR NEW.attempt_id IS NOT OLD.attempt_id OR NEW.attempt_runtime IS NOT OLD.attempt_runtime))
 BEGIN SELECT RAISE(ABORT,'native stream origin is immutable'); END;
CREATE TRIGGER native_stream_attempt_binding BEFORE UPDATE ON native_stream_operations
 WHEN NEW.effect_started=1 AND NOT EXISTS(SELECT 1 FROM operation_effects e
 WHERE e.id=NEW.effect_id AND e.operation_record_id=NEW.operation_record_id
 AND e.provider='native-stream' AND e.idempotency_mode='none' AND e.submission_generation=1)
 BEGIN SELECT RAISE(ABORT,'native stream attempt binding mismatch'); END;
CREATE TRIGGER native_stream_retained BEFORE DELETE ON native_stream_operations
 BEGIN SELECT RAISE(ABORT,'native stream identity remains reserved'); END;
CREATE TRIGGER native_stream_record_immutable BEFORE UPDATE ON operations
 WHEN EXISTS(SELECT 1 FROM native_stream_operations n WHERE n.operation_record_id=OLD.record_id)
 AND (NEW.expires_at IS NOT OLD.expires_at OR NEW.ledger_version!=OLD.ledger_version
 OR (OLD.state IN ('completed','cancelled','failed') AND (NEW.state!=OLD.state OR NEW.result_json IS NOT OLD.result_json OR NEW.problem_json IS NOT OLD.problem_json)))
 BEGIN SELECT RAISE(ABORT,'native stream record is immutable'); END;
PRAGMA user_version=40;
