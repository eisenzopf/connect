-- Fresh schema only. These are immutable captured physical facts, not current
-- permission. Rust still validates the exact predecessor capture, original
-- selection hash, current return actor and each surviving physical source.
CREATE VIEW native_private_replacement_captures AS
 SELECT r.operation_record_id,r.runtime_epoch,r.generation,r.candidate_connection_id,
 r.candidate_runtime_id,r.plan_json,o.connection_id AS predecessor_connection_id,
 o.conversation_id,o.session_id,s.observation_json,
 json_extract(s.observation_json,'$.projection.private_successor') AS lineage_json,
 json_extract(s.observation_json,'$.projection.private_successor.consultation_record_id') AS consultation_record_id,
 json_extract(s.observation_json,'$.projection.private_successor.target') AS target_json,
 l.credential_id,l.key_id,l.device_id,l.endpoint_id,l.participant_id,l.membership_epoch,l.source_expires_at
 FROM native_replacements r
 JOIN operations o ON o.record_id=r.operation_record_id AND o.state='completed'
 JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='commit' AND s.started=1
 JOIN operation_effects e ON e.id=s.effect_id AND e.operation_record_id=o.record_id
  AND e.provider='native-replacement-commit' AND e.state='confirmed' AND e.attempt=1 AND e.submission_generation=1
 JOIN native_replacement_lifetimes l ON l.operation_record_id=o.record_id
  AND l.attempt_runtime=r.runtime_epoch AND l.attempt_id=s.attempt_id AND l.effect_id=e.id AND l.generation=r.generation
 WHERE json_extract(o.result_json,'$.activation_state')='ready'
 AND json_type(s.observation_json,'$.projection.private_successor')='object';

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
 AND l.runtime_epoch=n.runtime_epoch AND l.audience_json=NEW.audience_json
 AND (
  (n.selection_json=NEW.context_selection_json AND o.connection_id=l.primary_connection_id
   AND NOT EXISTS(SELECT 1 FROM native_private_replacement_captures current
    WHERE current.consultation_record_id=n.operation_record_id AND current.runtime_epoch=n.runtime_epoch))
  OR EXISTS(SELECT 1 FROM native_private_replacement_captures current
   JOIN json_each(n.selection_json,'$.private') original
    ON json_extract(original.value,'$.connection_id')=l.primary_connection_id
   WHERE current.consultation_record_id=n.operation_record_id AND current.runtime_epoch=n.runtime_epoch
   AND current.conversation_id=o.conversation_id AND current.session_id=o.session_id
   AND current.candidate_connection_id=o.connection_id
   AND json_extract(current.lineage_json,'$.original_primary_connection_id')=l.primary_connection_id
   AND NOT EXISTS(SELECT 1 FROM native_private_replacement_captures newer
    WHERE newer.consultation_record_id=current.consultation_record_id AND newer.runtime_epoch=current.runtime_epoch
    AND newer.operation_record_id!=current.operation_record_id AND newer.generation>=current.generation)
   AND json_type(current.lineage_json,'$.predecessor')='object' AND json_type(current.lineage_json,'$.target')='object'
   AND json_extract(current.lineage_json,'$.predecessor.connection_id')=current.predecessor_connection_id
   AND json_extract(current.lineage_json,'$.predecessor.generation')=json_extract(current.plan_json,'$.source_generation')
   AND json_extract(current.lineage_json,'$.predecessor.runtime_connection_id')=json_extract(current.plan_json,'$.expected_preparation.source')
   AND json_extract(current.lineage_json,'$.predecessor.generation')+1=current.generation
   AND json_extract(current.target_json,'$.connection_id')=current.candidate_connection_id
   AND json_extract(current.target_json,'$.runtime_connection_id')=current.candidate_runtime_id
   AND json_extract(current.target_json,'$.generation')=current.generation
   AND json_extract(current.target_json,'$.credential_id')=current.credential_id
   AND json_extract(current.target_json,'$.key_id')=current.key_id
   AND json_extract(current.target_json,'$.device_id')=current.device_id
   AND json_extract(current.target_json,'$.endpoint_id')=current.endpoint_id
   AND json_extract(current.target_json,'$.participant_id')=current.participant_id
   AND json_extract(current.target_json,'$.membership_epoch')=current.membership_epoch
   AND json_extract(current.target_json,'$.physical_expires_at')=current.source_expires_at
   AND json_extract(current.target_json,'$.device_kind')=json_extract(current.plan_json,'$.target_device_kind')
   AND json_extract(current.target_json,'$.device_platform')=json_extract(current.plan_json,'$.target_device_platform')
   AND json_extract(current.target_json,'$.participant_id')=json_extract(original.value,'$.participant_id')
   AND json_extract(current.target_json,'$.membership_epoch')=json_extract(original.value,'$.membership_epoch')
   AND json_extract(current.lineage_json,'$.predecessor.participant_id')=json_extract(original.value,'$.participant_id')
   AND json_extract(current.lineage_json,'$.predecessor.membership_epoch')=json_extract(original.value,'$.membership_epoch')
   AND json_extract(current.observation_json,'$.movement.candidate')=current.candidate_runtime_id
   AND json_extract(current.observation_json,'$.movement.generation')=current.generation
   -- A retired latest successor remains a historical return source. Fresh
   -- authority separately requires its exact recorded departure; no live row
   -- requirement here may silently substitute the older physical owner.
   AND EXISTS(SELECT 1 FROM native_admission_sources predecessor
    JOIN native_connection_lifetimes life ON life.operation_record_id=predecessor.operation_record_id
     AND life.connection_id=predecessor.connection_id AND life.runtime_epoch=predecessor.attempt_runtime AND life.generation=predecessor.generation
    JOIN operation_effects effect ON effect.id=predecessor.effect_id AND effect.operation_record_id=predecessor.operation_record_id AND effect.state='confirmed'
    JOIN connection_offers media ON media.connection_id=predecessor.connection_id
    WHERE predecessor.operation_record_id=json_extract(current.lineage_json,'$.predecessor_record_id')
    AND predecessor.state='completed' AND predecessor.attempt_runtime=current.runtime_epoch
    AND predecessor.connection_id=current.predecessor_connection_id
    AND predecessor.generation=json_extract(current.lineage_json,'$.predecessor.generation')
    AND media.runtime_connection_id=json_extract(current.lineage_json,'$.predecessor.runtime_connection_id'))
   AND (
    (current.predecessor_connection_id=l.primary_connection_id
     AND json_extract(current.lineage_json,'$.predecessor')=original.value
     AND json_type(current.lineage_json,'$.predecessor_capture_sha256')='null')
    OR EXISTS(SELECT 1 FROM native_private_replacement_captures predecessor
     WHERE predecessor.operation_record_id=json_extract(current.lineage_json,'$.predecessor_record_id')
     AND predecessor.consultation_record_id=current.consultation_record_id AND predecessor.runtime_epoch=current.runtime_epoch
     AND predecessor.generation+1=current.generation
     AND predecessor.target_json=json_extract(current.lineage_json,'$.predecessor')
     AND json_extract(predecessor.lineage_json,'$.original_primary_connection_id')=l.primary_connection_id
     AND json_extract(predecessor.lineage_json,'$.original_selection_sha256')=json_extract(current.lineage_json,'$.original_selection_sha256')
     AND json_extract(predecessor.lineage_json,'$.private_expires_at')>=json_extract(current.lineage_json,'$.private_expires_at')))
   -- Substitute precisely one original primary slot. This also preserves the
   -- remote, private partner, outside slots, ordering and all other fields.
   AND json_set(n.selection_json,'$.private['||original.key||']',json(current.target_json))=NEW.context_selection_json))
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
PRAGMA user_version=117;
