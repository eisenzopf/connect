-- Configured AI consent is separate from the caller's consultation authority.
-- Both ordinary Session receipts and the exact causal revision delta commit
-- before any private native/provider IO. No unrelated revision is absorbed.
CREATE TABLE native_consultation_joins (
 operation_record_id TEXT PRIMARY KEY REFERENCES operations(record_id),
 runtime_epoch TEXT NOT NULL,
 original_digest TEXT NOT NULL,
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL,
 delegate_credential_id TEXT NOT NULL REFERENCES credentials(id),
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 endpoint_revision INTEGER NOT NULL,
 base_revision INTEGER NOT NULL,
 effective_revision INTEGER NOT NULL CHECK(effective_revision=base_revision+2),
 invite_record_id TEXT NOT NULL UNIQUE REFERENCES operations(record_id),
 accept_record_id TEXT NOT NULL UNIQUE REFERENCES operations(record_id),
 joined_at INTEGER NOT NULL
);
CREATE TRIGGER consultation_join_binding BEFORE INSERT ON native_consultation_joins
 WHEN NOT EXISTS(SELECT 1 FROM operations o
 JOIN operations i ON i.record_id=NEW.invite_record_id AND i.owner_id=o.owner_id AND i.command_type='session.invite' AND i.state='completed' AND i.session_id=o.session_id AND i.expected_revision=NEW.base_revision
 JOIN operations a ON a.record_id=NEW.accept_record_id AND a.command_type='session.accept' AND a.state='completed' AND a.session_id=o.session_id AND a.expected_revision=NEW.base_revision+1
 JOIN operation_members m ON m.operation_record_id=o.record_id AND m.participant_id=NEW.participant_id AND m.membership_epoch=NEW.membership_epoch
 JOIN credentials k ON k.id=NEW.delegate_credential_id AND k.identity_id=m.identity_id AND k.identity_id=a.owner_id
 JOIN endpoints e ON e.id=NEW.endpoint_id AND e.identity_id=m.identity_id AND e.revision=NEW.endpoint_revision AND e.transport='quic' AND e.verification_state='verified'
 JOIN sessions s ON s.id=o.session_id AND s.topology_revision=NEW.effective_revision
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='session.consult' AND o.state='accepted' AND o.digest=NEW.original_digest AND o.expected_revision=NEW.base_revision)
 BEGIN SELECT RAISE(ABORT,'consultation join requires original source and both receipts'); END;
CREATE TRIGGER consultation_join_immutable BEFORE UPDATE ON native_consultation_joins BEGIN SELECT RAISE(ABORT,'consultation join is immutable'); END;
CREATE TRIGGER consultation_join_retained BEFORE DELETE ON native_consultation_joins BEGIN SELECT RAISE(ABORT,'consultation join receipt remains reserved'); END;
PRAGMA user_version=53;
