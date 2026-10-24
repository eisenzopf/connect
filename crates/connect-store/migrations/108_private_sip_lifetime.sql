-- A private lifetime captures its original typed candidate, never a public SIP
-- sibling or a fabricated native admission. Current IO authority remains separate.
DROP TRIGGER native_private_lifetime_binding;
CREATE TRIGGER native_private_lifetime_binding BEFORE INSERT ON native_private_lifetimes
 WHEN NEW.restored_at IS NOT NULL OR NOT EXISTS(
 SELECT 1 FROM native_consultation_attempts n
 JOIN consultation_candidate_bindings l ON l.operation_record_id=n.operation_record_id
 JOIN operations o ON o.record_id=n.operation_record_id
 JOIN operation_effects e ON e.id=n.effect_id
 WHERE n.operation_record_id=NEW.operation_record_id AND n.runtime_epoch=NEW.runtime_epoch
 AND l.runtime_epoch=n.runtime_epoch AND l.original_digest=o.digest
 AND l.primary_connection_id=o.connection_id
 AND n.selection_json=NEW.selection_json AND l.audience_json=NEW.audience_json
 AND n.started=1 AND n.applied_json IS NOT NULL AND n.decoded_audio_observed=1
 AND n.media_failure IS NULL AND e.state='confirmed'
 AND o.state IN ('committing','unknown')
 AND (l.candidate_kind='native' OR (l.candidate_kind='sip'
   AND json_array_length(NEW.selection_json,'$.private')=2
   AND EXISTS(SELECT 1 FROM json_each(NEW.selection_json,'$.private') selected
     JOIN initial_telephone_operations child ON child.operation_record_id=json_extract(selected.value,'$.record_id')
     JOIN operations call ON call.record_id=child.operation_record_id AND call.state='completed'
     JOIN initial_telephone_answers a ON a.operation_record_id=child.operation_record_id
     JOIN initial_telephone_private_acceptances own ON own.operation_record_id=child.operation_record_id
     JOIN operation_effects observed ON observed.id=child.effect_id AND observed.state='confirmed'
     WHERE json_extract(selected.value,'$.source_kind')='private-initial-sip'
     AND json_extract(selected.value,'$.consultation_record_id')=l.operation_record_id
     AND json_extract(selected.value,'$.connection_id')=l.connection_id
     AND json_extract(selected.value,'$.generation')=1
     AND child.parent_kind='consultation' AND child.parent_record_id=l.operation_record_id
     AND child.attempt_runtime=n.runtime_epoch AND child.signaling_started=1
     AND call.connection_id=l.connection_id
     AND json_extract(selected.value,'$.runtime_connection_id')=child.runtime_connection_id
     AND json_extract(selected.value,'$.sip_call_id')=child.sip_call_id
     AND own.consultation_record_id=l.operation_record_id AND own.endpoint_id=l.endpoint_id
     AND own.endpoint_revision=l.endpoint_revision
     AND json_extract(selected.value,'$.endpoint_id')=own.endpoint_id
     AND json_extract(selected.value,'$.endpoint_revision')=own.endpoint_revision
     AND json_extract(selected.value,'$.endpoint_credential_id')=own.endpoint_credential_id
     AND json_extract(selected.value,'$.participant_id')=own.participant_id
     AND json_extract(selected.value,'$.membership_epoch')=own.membership_epoch
     AND a.accepted=1 AND a.acceptance_kind='private-connection' AND a.acceptance_record_id=child.operation_record_id
     AND a.connected_at IS NOT NULL AND a.retired_at IS NULL
     AND json_extract(a.connected_json,'$.private_audio')=1
     AND json_extract(a.connected_json,'$.private_consultation_record_id')=l.operation_record_id
     AND json_extract(a.connected_json,'$.connection_id')=l.connection_id
     AND json_extract(a.connected_json,'$.runtime_connection_id')=child.runtime_connection_id
     AND json_extract(a.connected_json,'$.call_id')=child.sip_call_id
     AND json_extract(a.connected_json,'$.stream_revision')=2)
   AND EXISTS(SELECT 1 FROM json_each(NEW.selection_json,'$.private') selected
     WHERE json_extract(selected.value,'$.connection_id')=l.primary_connection_id
     AND json_extract(selected.value,'$.source_kind') IS NULL))))
 BEGIN SELECT RAISE(ABORT,'private lifetime requires actual decoded typed application'); END;
PRAGMA user_version=108;
