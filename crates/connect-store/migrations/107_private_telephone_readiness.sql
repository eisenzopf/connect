-- A private ready fact belongs only to the original quarantined admission.
-- It cannot enter the public Session readiness/reconnect coverage.
CREATE TRIGGER private_telephone_connected_binding BEFORE UPDATE ON initial_telephone_answers
 WHEN NEW.connected_json IS NOT NULL
 AND EXISTS(SELECT 1 FROM initial_telephone_operations n WHERE n.operation_record_id=NEW.operation_record_id AND n.parent_kind='consultation')
 AND NOT EXISTS(SELECT 1 FROM initial_telephone_operations n
 JOIN operations child ON child.record_id=n.operation_record_id
 JOIN initial_telephone_private_acceptances own ON own.operation_record_id=n.operation_record_id
 WHERE n.operation_record_id=NEW.operation_record_id
 AND NEW.accepted=1 AND NEW.acceptance_kind='private-connection' AND NEW.acceptance_record_id=n.operation_record_id
 AND own.consultation_record_id=n.parent_record_id AND own.codec=NEW.codec
 AND json_extract(NEW.connected_json,'$.private_audio')=1
 AND json_extract(NEW.connected_json,'$.private_consultation_record_id')=n.parent_record_id
 AND json_extract(NEW.connected_json,'$.connection_id')=child.connection_id
 AND json_extract(NEW.connected_json,'$.runtime_connection_id')=n.runtime_connection_id
 AND json_extract(NEW.connected_json,'$.call_id')=n.sip_call_id
 AND json_extract(NEW.connected_json,'$.participant_id')=own.participant_id
 AND json_extract(NEW.connected_json,'$.membership_epoch')=own.membership_epoch
 AND json_extract(NEW.connected_json,'$.endpoint_id')=own.endpoint_id
 AND json_extract(NEW.connected_json,'$.device_id')=json_extract(n.source_json,'$.device_id')
 AND json_extract(NEW.connected_json,'$.generation')=1
 AND json_extract(NEW.connected_json,'$.stream_revision')=2
 AND json_extract(NEW.connected_json,'$.participant_revision')>0
 AND json_extract(NEW.connected_json,'$.codec')=NEW.codec
 AND json_array_length(NEW.connected_json,'$.streams')=1
 AND json_extract(NEW.connected_json,'$.streams[0]')='telephone-audio')
 BEGIN SELECT RAISE(ABORT,'private telephone ready fact has no original quarantine binding'); END;
PRAGMA user_version=107;
