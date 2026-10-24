-- Restore only the original typed applied context. A SIP candidate may already
-- be retired; historical captured identity, not fresh media authority, binds it.
DROP TRIGGER native_return_binding;
CREATE TRIGGER native_return_binding BEFORE INSERT ON native_consultation_returns
 WHEN NOT EXISTS(SELECT 1 FROM operations o
 JOIN native_consultation_attempts n ON n.operation_record_id=NEW.consultation_record_id
 JOIN consultation_candidate_bindings l ON l.operation_record_id=n.operation_record_id
 JOIN operation_effects e ON e.id=n.effect_id AND e.state='confirmed'
 JOIN credentials k ON k.id=NEW.credential_id AND k.identity_id=o.owner_id AND k.revoked=0
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='session.consult.return'
 AND o.state='accepted' AND o.digest=NEW.original_digest
 AND n.started=1 AND n.applied_json IS NOT NULL AND n.runtime_epoch=NEW.runtime_epoch
 AND l.runtime_epoch=n.runtime_epoch
 AND n.selection_json=NEW.context_selection_json AND l.audience_json=NEW.audience_json
 AND (l.candidate_kind='native' OR (l.candidate_kind='sip'
   AND EXISTS(SELECT 1 FROM json_each(NEW.context_selection_json,'$.private') selected
     JOIN initial_telephone_operations child ON child.operation_record_id=json_extract(selected.value,'$.record_id')
     JOIN operations call ON call.record_id=child.operation_record_id AND call.connection_id=l.connection_id
     JOIN initial_telephone_answers a ON a.operation_record_id=child.operation_record_id
     JOIN initial_telephone_private_acceptances own ON own.operation_record_id=child.operation_record_id
     WHERE json_extract(selected.value,'$.source_kind')='private-initial-sip'
     AND json_extract(selected.value,'$.consultation_record_id')=l.operation_record_id
     AND json_extract(selected.value,'$.connection_id')=l.connection_id
     AND json_extract(selected.value,'$.generation')=1
     AND child.parent_kind='consultation' AND child.parent_record_id=l.operation_record_id
     AND child.attempt_runtime=n.runtime_epoch AND child.signaling_started=1
     AND json_extract(selected.value,'$.runtime_connection_id')=child.runtime_connection_id
     AND json_extract(selected.value,'$.sip_call_id')=child.sip_call_id
     AND own.consultation_record_id=l.operation_record_id
     AND json_extract(selected.value,'$.endpoint_id')=own.endpoint_id
     AND json_extract(selected.value,'$.endpoint_revision')=own.endpoint_revision
     AND json_extract(selected.value,'$.endpoint_credential_id')=own.endpoint_credential_id
     AND json_extract(selected.value,'$.participant_id')=own.participant_id
     AND json_extract(selected.value,'$.membership_epoch')=own.membership_epoch
     AND a.accepted=1 AND a.acceptance_kind='private-connection'
     AND a.acceptance_record_id=child.operation_record_id AND a.connected_at IS NOT NULL
     AND json_extract(a.connected_json,'$.private_audio')=1
     AND json_extract(a.connected_json,'$.private_consultation_record_id')=l.operation_record_id
     AND json_extract(a.connected_json,'$.connection_id')=l.connection_id
     AND json_extract(a.connected_json,'$.runtime_connection_id')=child.runtime_connection_id
     AND json_extract(a.connected_json,'$.call_id')=child.sip_call_id))))
 BEGIN SELECT RAISE(ABORT,'return context has no original typed application'); END;
PRAGMA user_version=109;
