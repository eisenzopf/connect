-- Native peer media records retain their actual verified transport. Both
-- canonical native adapters obey the same current source/receipt constraints.
DROP TRIGGER consultation_join_binding;
CREATE TRIGGER consultation_join_binding BEFORE INSERT ON native_consultation_joins
 WHEN NOT EXISTS(SELECT 1 FROM operations o
 JOIN operations i ON i.record_id=NEW.invite_record_id AND i.owner_id=o.owner_id AND i.command_type='session.invite' AND i.state='completed' AND i.session_id=o.session_id AND i.expected_revision=NEW.base_revision
 JOIN operations a ON a.record_id=NEW.accept_record_id AND a.command_type='session.accept' AND a.state='completed' AND a.session_id=o.session_id AND a.expected_revision=NEW.base_revision+1
 JOIN operation_members m ON m.operation_record_id=o.record_id AND m.participant_id=NEW.participant_id AND m.membership_epoch=NEW.membership_epoch
 JOIN credentials k ON k.id=NEW.delegate_credential_id AND k.identity_id=m.identity_id AND k.identity_id=a.owner_id
 JOIN endpoints e ON e.id=NEW.endpoint_id AND e.identity_id=m.identity_id AND e.revision=NEW.endpoint_revision AND e.transport IN ('quic','webtransport') AND e.verification_state='verified'
 JOIN sessions s ON s.id=o.session_id AND s.topology_revision=NEW.effective_revision
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='session.consult' AND o.state='accepted' AND o.digest=NEW.original_digest AND o.expected_revision=NEW.base_revision)
 BEGIN SELECT RAISE(ABORT,'consultation join requires original source and both receipts'); END;
PRAGMA user_version=111;
