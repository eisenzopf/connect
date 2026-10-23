-- Fresh schema only. A replacement keeps its real root operation and target
-- incarnation; it never fabricates an initial offer or a generation-one route.
CREATE TABLE native_replacement_lifetimes (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_replacements(operation_record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 key_id TEXT NOT NULL REFERENCES peer_signing_keys(key_id),
 device_id TEXT NOT NULL,
 endpoint_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL,
 generation INTEGER NOT NULL CHECK(generation BETWEEN 2 AND 9007199254740991),
 resources_json TEXT NOT NULL CHECK(json_valid(resources_json)),
 source_expires_at INTEGER NOT NULL,
 attempt_runtime TEXT NOT NULL,
 attempt_id TEXT NOT NULL UNIQUE,
 effect_id TEXT NOT NULL UNIQUE REFERENCES operation_effects(id)
);
CREATE TRIGGER native_replacement_lifetime_binding BEFORE INSERT ON native_replacement_lifetimes
 WHEN NOT EXISTS(SELECT 1 FROM native_replacements n JOIN operations o ON o.record_id=n.operation_record_id
 JOIN native_replacement_steps commit_step ON commit_step.operation_record_id=o.record_id AND commit_step.phase='commit'
 JOIN operation_effects commit_effect ON commit_effect.id=commit_step.effect_id AND commit_effect.state='confirmed'
 JOIN native_replacement_steps start ON start.operation_record_id=o.record_id AND start.phase='start'
 JOIN operation_effects start_effect ON start_effect.id=start.effect_id AND start_effect.state='confirmed'
 JOIN connections c ON c.id=n.candidate_connection_id
 JOIN credentials credential ON credential.id=NEW.credential_id AND credential.identity_id=c.identity_id
 JOIN credential_signing_keys key_binding ON key_binding.credential_id=NEW.credential_id AND key_binding.key_id=NEW.key_id
 JOIN peer_signing_keys key ON key.key_id=key_binding.key_id AND key.device_id=NEW.device_id AND key.identity_id=c.identity_id
 WHERE n.operation_record_id=NEW.operation_record_id AND o.state='completed'
 AND n.runtime_epoch=NEW.attempt_runtime AND commit_step.attempt_id=NEW.attempt_id
 AND commit_step.effect_id=NEW.effect_id AND commit_step.started=1 AND commit_step.observation_json IS NOT NULL
 AND json_extract(start_effect.observation_json,'$.credential_id')=NEW.credential_id
 AND json_extract(start_effect.observation_json,'$.key_id')=NEW.key_id
 AND json_extract(start_effect.observation_json,'$.device_id')=NEW.device_id
 AND json_extract(start_effect.observation_json,'$.endpoint_id')=NEW.endpoint_id
 AND c.generation=NEW.generation AND c.generation=n.generation AND c.state='ready'
 AND c.observed_runtime_id=NEW.attempt_runtime AND c.readiness_json IS NOT NULL
 AND c.participant_id=NEW.participant_id AND c.membership_epoch=NEW.membership_epoch
 AND c.endpoint_id=NEW.endpoint_id AND credential.expires_at>=NEW.source_expires_at
 AND key.expires_at>=NEW.source_expires_at)
 BEGIN SELECT RAISE(ABORT,'native replacement lifetime mismatch'); END;
CREATE TRIGGER native_replacement_lifetime_immutable BEFORE UPDATE ON native_replacement_lifetimes
 BEGIN SELECT RAISE(ABORT,'native replacement lifetime origin is immutable'); END;
CREATE TRIGGER native_replacement_lifetime_retained BEFORE DELETE ON native_replacement_lifetimes
 BEGIN SELECT RAISE(ABORT,'native replacement lifetime remains reserved'); END;
CREATE VIEW native_admission_sources AS
 SELECT n.operation_record_id,n.preparation_record_id,n.credential_id,n.key_id,n.device_id,n.endpoint_id,n.participant_id,n.membership_epoch,n.generation,n.resources_json,n.source_expires_at,n.terminal_reserve,n.attempt_id,n.attempt_runtime,n.effect_id,n.effect_started,o.record_id,o.operation_id,o.owner_id,o.tenant_id,o.conversation_id,o.session_id,o.connection_id,o.expected_revision,o.state
 FROM native_offer_operations n JOIN operations o ON o.record_id=n.operation_record_id
 UNION ALL
 SELECT n.operation_record_id,NULL,n.credential_id,n.key_id,n.device_id,n.endpoint_id,n.participant_id,n.membership_epoch,n.generation,n.resources_json,n.source_expires_at,json_array_length(n.resources_json)+1,n.attempt_id,n.attempt_runtime,n.effect_id,1,
 o.record_id,o.operation_id,c.identity_id,c.tenant_id,o.conversation_id,o.session_id,c.id,0,o.state
 FROM native_replacement_lifetimes n JOIN operations o ON o.record_id=n.operation_record_id
 JOIN native_replacements r ON r.operation_record_id=n.operation_record_id JOIN connections c ON c.id=r.candidate_connection_id;
DROP TRIGGER native_lifetime_binding;
CREATE TRIGGER native_lifetime_binding BEFORE INSERT ON native_connection_lifetimes
 WHEN NOT EXISTS(SELECT 1 FROM native_admission_sources n JOIN connections c ON c.id=n.connection_id
 JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed'
 WHERE n.operation_record_id=NEW.operation_record_id AND n.state='completed' AND n.effect_started=1
 AND n.attempt_runtime=NEW.runtime_epoch AND c.id=NEW.connection_id AND c.generation=NEW.generation
 AND c.state='ready' AND c.observed_runtime_id=NEW.runtime_epoch AND c.readiness_json IS NOT NULL)
 BEGIN SELECT RAISE(ABORT,'native lifetime binding mismatch'); END;

DROP TRIGGER native_stream_binding;
CREATE TRIGGER native_stream_binding BEFORE INSERT ON native_stream_operations
 WHEN NOT EXISTS(SELECT 1 FROM operations o
 JOIN native_admission_sources parent ON parent.record_id=NEW.parent_record_id
 JOIN native_admission_sources n ON n.operation_record_id=parent.record_id
 JOIN native_connection_lifetimes life ON life.operation_record_id=parent.record_id
 JOIN connections c ON c.id=parent.connection_id
 JOIN credentials credential ON credential.id=NEW.credential_id
 JOIN credential_signing_keys binding ON binding.credential_id=credential.id AND binding.key_id=NEW.key_id
 JOIN peer_signing_keys key ON key.key_id=binding.key_id
 WHERE o.record_id=NEW.operation_record_id AND o.ledger_version=0 AND o.state='accepted'
 AND o.command_type IN ('stream.subscribe','stream.unsubscribe','connection.end','message.send','stream.reconcile','connection.update')
 AND o.owner_id=parent.owner_id AND o.tenant_id=parent.tenant_id
 AND o.conversation_id=parent.conversation_id AND o.session_id=parent.session_id AND o.connection_id=parent.connection_id
 AND parent.state='completed' AND life.retired=0 AND life.runtime_epoch=n.attempt_runtime
 AND c.state IN ('ready','active') AND c.generation=NEW.generation AND c.generation=life.generation
 AND c.observed_runtime_id=life.runtime_epoch AND c.readiness_json IS NOT NULL
 AND (o.expected_revision BETWEEN 0 AND 9007199254740991 OR (o.command_type IN ('connection.end','message.send') AND o.expected_revision IS NULL))
 AND json_extract(o.intent_json,'$.operation_id')=o.operation_id
 AND json_extract(o.intent_json,'$.conversation_id')=o.conversation_id
 AND json_extract(o.intent_json,'$.session_id')=o.session_id
 AND json_extract(o.intent_json,'$.connection_id')=o.connection_id
 AND json_extract(o.intent_json,'$.expected_revision') IS o.expected_revision
 AND json_extract(o.intent_json,'$.command')=o.command_type
 AND NEW.credential_id=n.credential_id AND NEW.key_id=n.key_id AND NEW.device_id=n.device_id
 AND NEW.participant_id=n.participant_id AND NEW.membership_epoch=n.membership_epoch
 AND NEW.resources_json=n.resources_json AND NEW.source_expires_at<=n.source_expires_at
 AND credential.identity_id=o.owner_id AND key.identity_id=o.owner_id AND key.device_id=NEW.device_id
 AND key.not_before<=o.created_at AND key.expires_at>=NEW.source_expires_at
 AND credential.expires_at>=NEW.source_expires_at AND o.expires_at<=NEW.source_expires_at
 AND (o.command_type!='stream.reconcile' OR EXISTS(SELECT 1 FROM operations selection JOIN native_stream_operations root ON root.operation_record_id=selection.record_id JOIN operation_effects selected ON selected.id=root.effect_id AND selected.operation_record_id=selection.record_id
  WHERE selection.record_id=json_extract(o.intent_json,'$.payload.subscription_record_id') AND root.parent_record_id=NEW.parent_record_id AND root.closed=1 AND selection.state='completed' AND selected.state='confirmed' AND selection.command_type IN ('stream.subscribe','stream.unsubscribe')
  AND selection.owner_id=o.owner_id AND root.credential_id=NEW.credential_id AND root.key_id=NEW.key_id AND root.device_id=NEW.device_id))
 AND NEW.closed=0 AND NEW.effect_started=0)
 BEGIN SELECT RAISE(ABORT,'native stream source binding mismatch'); END;

DROP TRIGGER native_data_write_binding;
CREATE TRIGGER native_data_write_binding BEFORE INSERT ON native_data_writes
 WHEN NOT EXISTS(SELECT 1 FROM native_stream_operations n
 JOIN operations o ON o.record_id=n.operation_record_id
 JOIN operation_effects e ON e.id=n.effect_id AND e.operation_record_id=o.record_id
 JOIN native_admission_sources target ON target.record_id=NEW.target_record_id
 JOIN native_admission_sources t ON t.operation_record_id=target.record_id
 JOIN native_data_recipients r ON r.operation_record_id=o.record_id
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='message.send'
 AND n.effect_started=1 AND n.attempt_runtime=NEW.runtime_epoch
 AND e.state IN ('submitting','confirmed','unknown')
 AND target.conversation_id=o.conversation_id AND target.session_id=o.session_id
 AND t.participant_id=r.participant_id AND t.membership_epoch=r.membership_epoch
 AND t.attempt_runtime=NEW.runtime_epoch AND t.effect_started=1
 AND NEW.disposition IS NULL)
 BEGIN SELECT RAISE(ABORT,'native data writer origin mismatch'); END;

DROP TRIGGER connection_lineage_generation;
CREATE TRIGGER connection_lineage_generation BEFORE INSERT ON connections
 WHEN NEW.replaces IS NOT NULL AND NOT EXISTS(
 SELECT 1 FROM connections old WHERE old.id=NEW.replaces AND
 COALESCE((SELECT min(n.generation) FROM native_admission_sources n WHERE n.connection_id=old.id AND n.state='completed'),old.generation)<NEW.generation)
 BEGIN SELECT RAISE(ABORT,'invalid connection lineage generation'); END;
PRAGMA user_version=57;
