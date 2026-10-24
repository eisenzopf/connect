-- Fresh schema initialization only: older stores are refused, never upgraded.
-- Replace the invitation-only shape with a typed immutable parent.
DROP TABLE initial_telephone_operations;
CREATE TABLE initial_telephone_operations (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 parent_record_id TEXT NOT NULL REFERENCES operations(record_id),
 parent_kind TEXT NOT NULL CHECK(parent_kind IN ('invitation','consultation')),
 caller_credential_id TEXT NOT NULL REFERENCES credentials(id),
 target_participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 source_json TEXT NOT NULL CHECK(json_valid(source_json)),
 policy_json TEXT NOT NULL CHECK(json_valid(policy_json)),
 allocation_started INTEGER NOT NULL DEFAULT 0 CHECK(allocation_started IN (0,1)),
 attempt_id TEXT UNIQUE,
 attempt_runtime TEXT,
 runtime_connection_id TEXT UNIQUE,
 sip_call_id TEXT UNIQUE,
 signaling_started INTEGER NOT NULL DEFAULT 0 CHECK(signaling_started IN (0,1)),
 effect_id TEXT UNIQUE REFERENCES operation_effects(id),
 CHECK((allocation_started=0 AND attempt_id IS NULL AND attempt_runtime IS NULL)
    OR (allocation_started=1 AND attempt_id IS NOT NULL AND attempt_runtime IS NOT NULL)),
 CHECK((runtime_connection_id IS NULL)=(sip_call_id IS NULL)),
 CHECK(runtime_connection_id IS NULL OR allocation_started=1),
 CHECK((signaling_started=0 AND effect_id IS NULL)
    OR (signaling_started=1 AND effect_id IS NOT NULL AND runtime_connection_id IS NOT NULL)),
 UNIQUE(parent_record_id,target_participant_id)
);
CREATE TRIGGER initial_telephone_binding BEFORE INSERT ON initial_telephone_operations
 WHEN NOT EXISTS(SELECT 1 FROM operations child
 JOIN operations parent ON parent.record_id=NEW.parent_record_id
 JOIN credentials caller ON caller.id=NEW.caller_credential_id AND caller.identity_id=parent.owner_id AND caller.revoked=0
 JOIN connections conn ON conn.id=child.connection_id AND conn.session_id=child.session_id
 WHERE child.record_id=NEW.operation_record_id AND child.command_type='connection.offer'
 AND child.owner_id=parent.owner_id AND child.tenant_id=parent.tenant_id
 AND child.conversation_id=parent.conversation_id AND child.session_id=parent.session_id
 AND json_extract(NEW.source_json,'$.parent.kind')=NEW.parent_kind AND json_extract(NEW.source_json,'$.parent.record_id')=NEW.parent_record_id
 AND ((NEW.parent_kind='invitation' AND parent.command_type='session.invite' AND parent.state='completed' AND EXISTS(SELECT 1 FROM operation_control_receipts r WHERE r.operation_record_id=parent.record_id))
 OR (NEW.parent_kind='consultation' AND parent.command_type='session.consult' AND parent.state='accepted' AND parent.lease_owner IS NULL AND parent.cancel_requested=0 AND NOT EXISTS(SELECT 1 FROM operation_effects f WHERE f.operation_record_id=parent.record_id) AND EXISTS(
 SELECT 1 FROM sip_consultation_reservations r WHERE r.operation_record_id=parent.record_id AND r.connection_id=child.connection_id AND r.credential_id=NEW.caller_credential_id AND r.original_digest=parent.digest
 AND r.policy_json=NEW.policy_json AND r.endpoint_id=NEW.endpoint_id AND r.participant_id=NEW.target_participant_id
 AND json_extract(NEW.source_json,'$.conversation_id')=parent.conversation_id AND json_extract(NEW.source_json,'$.session_id')=parent.session_id
 AND json_extract(NEW.source_json,'$.caller.identity_id')=parent.owner_id
 AND json_extract(NEW.source_json,'$.target.participant_id')=r.participant_id AND json_extract(NEW.source_json,'$.target.membership_epoch')=r.membership_epoch
 AND json_extract(NEW.source_json,'$.endpoint_id')=r.endpoint_id AND json_extract(NEW.source_json,'$.endpoint_revision')=r.endpoint_revision AND json_extract(NEW.source_json,'$.device_id')=r.device_id
 AND json_array_length(NEW.source_json,'$.audience')=2
 AND NOT EXISTS(SELECT 1 FROM json_each(NEW.source_json,'$.audience') a WHERE NOT EXISTS(
 SELECT 1 FROM json_each(r.audience_json) b WHERE json_extract(a.value,'$.identity_id')=json_extract(b.value,'$[0]') AND json_extract(a.value,'$.participant_id')=json_extract(b.value,'$[1]') AND json_extract(a.value,'$.membership_epoch')=json_extract(b.value,'$[2]')))
 AND (SELECT count(DISTINCT json_extract(a.value,'$.participant_id')) FROM json_each(NEW.source_json,'$.audience') a)=2
 AND json_extract(NEW.source_json,'$.parent.kind')='consultation' AND json_extract(NEW.source_json,'$.parent.record_id')=parent.record_id AND json_extract(NEW.source_json,'$.parent.connection_id')=child.connection_id)))
 AND conn.participant_id=NEW.target_participant_id AND conn.endpoint_id=NEW.endpoint_id
 AND conn.generation=1 AND conn.state='preparing' AND NEW.allocation_started=0)
 BEGIN SELECT RAISE(ABORT,'initial telephone source binding mismatch'); END;
CREATE TRIGGER initial_telephone_immutable BEFORE UPDATE ON initial_telephone_operations
 WHEN NEW.operation_record_id!=OLD.operation_record_id
 OR NEW.parent_record_id!=OLD.parent_record_id OR NEW.parent_kind!=OLD.parent_kind
 OR NEW.caller_credential_id!=OLD.caller_credential_id
 OR NEW.target_participant_id!=OLD.target_participant_id OR NEW.endpoint_id!=OLD.endpoint_id
 OR NEW.source_json!=OLD.source_json OR NEW.policy_json!=OLD.policy_json
 OR NEW.allocation_started<OLD.allocation_started OR NEW.signaling_started<OLD.signaling_started
 OR (OLD.attempt_id IS NOT NULL AND NEW.attempt_id IS NOT OLD.attempt_id)
 OR (OLD.attempt_runtime IS NOT NULL AND NEW.attempt_runtime IS NOT OLD.attempt_runtime)
 OR (OLD.runtime_connection_id IS NOT NULL AND NEW.runtime_connection_id IS NOT OLD.runtime_connection_id)
 OR (OLD.sip_call_id IS NOT NULL AND NEW.sip_call_id IS NOT OLD.sip_call_id)
 OR (OLD.effect_id IS NOT NULL AND NEW.effect_id IS NOT OLD.effect_id)
 BEGIN SELECT RAISE(ABORT,'initial telephone identity is immutable'); END;
CREATE TRIGGER initial_telephone_retained BEFORE DELETE ON initial_telephone_operations
 BEGIN SELECT RAISE(ABORT,'initial telephone attempt remains reserved'); END;
-- The same predicate owns live recovery and terminal-history purge selection.
CREATE VIEW unsettled_transition_operations AS
 SELECT p.operation_record_id FROM private_provider_contexts p WHERE p.started=1 AND p.closed_at IS NULL
 UNION ALL
 SELECT e.operation_record_id FROM operation_effects e
 WHERE e.state IN ('submitting','unknown') OR (e.state='confirmed' AND e.compensates_effect_id IS NULL AND NOT EXISTS(
 SELECT 1 FROM operation_effects cleanup WHERE cleanup.compensates_effect_id=e.id AND cleanup.state='confirmed'))
 UNION ALL
 SELECT n.parent_record_id FROM initial_telephone_operations n JOIN operations child ON child.record_id=n.operation_record_id
 JOIN connections conn ON conn.id=child.connection_id
 WHERE n.parent_kind='consultation' AND n.allocation_started=1 AND conn.state NOT IN ('retired','failed');
PRAGMA user_version=105;
