-- Register exact DTMF intent under the original physical native source.
-- This reserves durable control custody; it grants no recipient signal IO.
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
 AND o.command_type IN ('stream.subscribe','stream.unsubscribe','connection.end','message.send','stream.reconcile','connection.update','capability.advertise','dtmf.send')
 AND o.owner_id=parent.owner_id AND o.tenant_id=parent.tenant_id
 AND o.conversation_id=parent.conversation_id AND o.session_id=parent.session_id AND o.connection_id=parent.connection_id
 AND parent.state='completed' AND life.retired=0 AND life.runtime_epoch=n.attempt_runtime
 AND c.state IN ('ready','active') AND c.generation=NEW.generation AND c.generation=life.generation
 AND c.observed_runtime_id=life.runtime_epoch AND c.readiness_json IS NOT NULL
 AND (o.expected_revision BETWEEN 0 AND 9007199254740991 OR (o.command_type IN ('connection.end','message.send','dtmf.send') AND o.expected_revision IS NULL))
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
 AND (o.command_type!='capability.advertise' OR (
  json_type(o.intent_json,'$.by_participant')='text'
  AND json_extract(o.intent_json,'$.by_participant')=NEW.participant_id
  AND json_type(o.intent_json,'$.payload.by_participant')='text'
  AND json_extract(o.intent_json,'$.payload.by_participant')=NEW.participant_id
  AND json_type(o.intent_json,'$.payload.capabilities')='object'
  AND length(CAST(json_extract(o.intent_json,'$.payload.capabilities') AS BLOB))<=16384
  AND json_extract(o.intent_json,'$.payload.trigger') IN ('endpoint-change','operator-policy','renegotiation-requested','network-adapt')))
 AND (o.command_type!='dtmf.send' OR (
  json_type(o.intent_json,'$.payload.digits')='text'
  AND length(CAST(json_extract(o.intent_json,'$.payload.digits') AS BLOB)) BETWEEN 1 AND 32
  AND length(CAST(json_extract(o.intent_json,'$.payload.digits') AS BLOB))=length(json_extract(o.intent_json,'$.payload.digits'))
  AND json_extract(o.intent_json,'$.payload.digits') NOT GLOB '*[^0-9ABCDabcd*#]*'
  AND json_type(o.intent_json,'$.payload.duration_ms')='integer'
  AND ((json_extract(o.intent_json,'$.payload.method')='rfc4733' AND json_extract(o.intent_json,'$.payload.duration_ms') BETWEEN 40 AND 6000)
    OR (json_extract(o.intent_json,'$.payload.method')='info' AND json_extract(o.intent_json,'$.payload.duration_ms') BETWEEN 100 AND 5000))
  AND json_extract(o.intent_json,'$.payload.duration_ms')*length(json_extract(o.intent_json,'$.payload.digits'))+70*(length(json_extract(o.intent_json,'$.payload.digits'))-1)<=30000
  AND length(CAST(json_extract(o.intent_json,'$.payload') AS BLOB))<=32768))
 AND NEW.closed=0 AND NEW.effect_started=0)
 BEGIN SELECT RAISE(ABORT,'native stream source binding mismatch'); END;
PRAGMA user_version=83;
