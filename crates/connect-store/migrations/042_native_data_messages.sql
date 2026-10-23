-- Native queue admission shares operation custody and the durable Message noun.
DROP TRIGGER native_stream_binding;
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
 AND o.command_type IN ('stream.subscribe','stream.unsubscribe','connection.end','message.send')
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
 AND NEW.closed=0 AND NEW.effect_started=0)
 BEGIN SELECT RAISE(ABORT,'native stream source binding mismatch'); END;
CREATE TABLE native_data_messages (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_stream_operations(operation_record_id),
 message_id TEXT NOT NULL UNIQUE REFERENCES messages(id),
 source_stream_revision INTEGER NOT NULL CHECK(source_stream_revision BETWEEN 1 AND 9007199254740991)
);
CREATE TABLE native_data_recipients (
 operation_record_id TEXT NOT NULL REFERENCES native_data_messages(operation_record_id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 identity_id TEXT NOT NULL REFERENCES identities(id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 member_json TEXT NOT NULL CHECK(json_valid(member_json) AND json_extract(member_json,'$.participant_id')=participant_id AND json_extract(member_json,'$.identity_id')=identity_id AND json_extract(member_json,'$.membership_epoch')=membership_epoch),
 PRIMARY KEY(operation_record_id,participant_id)
);
CREATE TRIGGER native_data_message_binding BEFORE INSERT ON native_data_messages
 WHEN NOT EXISTS(SELECT 1 FROM operations o JOIN native_stream_operations n ON n.operation_record_id=o.record_id JOIN messages m ON m.id=NEW.message_id
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='message.send' AND o.state='accepted' AND n.effect_started=0
 AND m.conversation_id=o.conversation_id AND m.sender=o.owner_id AND m.sender_participant_id=n.participant_id
 AND m.id=json_extract(o.intent_json,'$.payload.msg_id') AND m.label=json_extract(o.intent_json,'$.payload.label')
 AND m.body=json_extract(o.intent_json,'$.payload.body') AND m.content_type=json_extract(o.intent_json,'$.payload.content_type')
 AND m.body_encoding=COALESCE(json_extract(o.intent_json,'$.payload.body_encoding'),'utf8'))
 BEGIN SELECT RAISE(ABORT,'native data message origin mismatch'); END;
CREATE TRIGGER native_data_recipient_binding BEFORE INSERT ON native_data_recipients
 WHEN NOT EXISTS(SELECT 1 FROM native_data_messages d JOIN operations o ON o.record_id=d.operation_record_id JOIN message_audiences a ON a.message_id=d.message_id AND a.conversation_id=o.conversation_id
 JOIN participants p ON p.participant_id=a.participant_id AND p.conversation_id=a.conversation_id AND p.identity_id=NEW.identity_id
 WHERE d.operation_record_id=NEW.operation_record_id AND o.state='accepted' AND a.participant_id=NEW.participant_id AND a.membership_epoch=NEW.membership_epoch)
 BEGIN SELECT RAISE(ABORT,'native data recipient origin mismatch'); END;
CREATE TRIGGER native_data_message_immutable BEFORE UPDATE ON native_data_messages
 BEGIN SELECT RAISE(ABORT,'native data message origin is immutable'); END;
CREATE TRIGGER native_data_message_retained BEFORE DELETE ON native_data_messages
 BEGIN SELECT RAISE(ABORT,'native data message identity remains reserved'); END;
CREATE TRIGGER native_data_recipient_immutable BEFORE UPDATE ON native_data_recipients
 BEGIN SELECT RAISE(ABORT,'native data original audience is immutable'); END;
CREATE TRIGGER native_data_recipient_retained BEFORE DELETE ON native_data_recipients
 BEGIN SELECT RAISE(ABORT,'native data original audience remains reserved'); END;
PRAGMA user_version=42;
