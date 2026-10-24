-- An own private SIP answer accepts only its original reserved Connection.
-- It never fabricates a public session.accept operation or presence receipt.
ALTER TABLE initial_telephone_answers ADD COLUMN acceptance_kind TEXT
 CHECK(acceptance_kind IN ('public-session','private-connection'));
CREATE TABLE initial_telephone_private_acceptances (
 operation_record_id TEXT PRIMARY KEY REFERENCES initial_telephone_answers(operation_record_id),
 consultation_record_id TEXT NOT NULL REFERENCES operations(record_id),
 endpoint_credential_id TEXT NOT NULL REFERENCES credentials(id),
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 endpoint_revision INTEGER NOT NULL,
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch>0),
 codec TEXT NOT NULL CHECK(length(codec) BETWEEN 1 AND 64),
 audience_json TEXT NOT NULL CHECK(json_valid(audience_json) AND json_array_length(audience_json)=2),
 accepted_at INTEGER NOT NULL
);
CREATE TRIGGER private_telephone_acceptance_binding BEFORE INSERT ON initial_telephone_private_acceptances
 WHEN NOT EXISTS(SELECT 1 FROM initial_telephone_operations n
 JOIN operations child ON child.record_id=n.operation_record_id
 JOIN initial_telephone_answers a ON a.operation_record_id=n.operation_record_id
 JOIN sip_consultation_reservations r ON r.operation_record_id=n.parent_record_id AND r.connection_id=child.connection_id
 JOIN operations parent ON parent.record_id=r.operation_record_id AND parent.digest=r.original_digest
 JOIN credentials k ON k.id=NEW.endpoint_credential_id AND k.identity_id=json_extract(n.source_json,'$.target.identity_id') AND k.revoked=0
 JOIN endpoints e ON e.id=NEW.endpoint_id AND e.revision=NEW.endpoint_revision AND e.identity_id=k.identity_id AND e.verification_state='verified'
 JOIN participants p ON p.participant_id=NEW.participant_id AND p.membership_epoch=NEW.membership_epoch AND p.identity_id=k.identity_id AND p.active=1
 WHERE n.operation_record_id=NEW.operation_record_id AND n.parent_kind='consultation'
 AND NEW.consultation_record_id=n.parent_record_id AND NEW.endpoint_credential_id=json_extract(n.policy_json,'$.endpoint_credential_id')
 AND NEW.endpoint_id=r.endpoint_id AND NEW.endpoint_revision=r.endpoint_revision
 AND NEW.participant_id=r.participant_id AND NEW.membership_epoch=r.membership_epoch
 AND NEW.codec=a.codec AND NEW.audience_json=json_extract(n.source_json,'$.audience')
 AND a.accepted=0 AND a.retired_at IS NULL AND a.connected_at IS NULL
 AND child.state IN ('accepted','preparing') AND child.cancel_requested=0
 AND parent.state='accepted' AND parent.cancel_requested=0 AND parent.lease_owner IS NULL
 AND NOT EXISTS(SELECT 1 FROM operation_effects f WHERE f.operation_record_id=parent.record_id))
 BEGIN SELECT RAISE(ABORT,'private telephone acceptance has no matching own original answer'); END;
CREATE TRIGGER private_telephone_acceptance_immutable BEFORE UPDATE ON initial_telephone_private_acceptances
 BEGIN SELECT RAISE(ABORT,'private telephone acceptance is immutable'); END;
CREATE TRIGGER private_telephone_acceptance_retained BEFORE DELETE ON initial_telephone_private_acceptances
 BEGIN SELECT RAISE(ABORT,'private telephone acceptance remains reserved'); END;
CREATE TRIGGER initial_telephone_answer_starts_unaccepted BEFORE INSERT ON initial_telephone_answers
 WHEN NEW.accepted!=0 OR NEW.acceptance_kind IS NOT NULL OR NEW.connected_at IS NOT NULL OR NEW.retired_at IS NOT NULL
 BEGIN SELECT RAISE(ABORT,'initial answer must precede consent and readiness'); END;
CREATE TRIGGER initial_telephone_acceptance_kind BEFORE UPDATE ON initial_telephone_answers
 WHEN (NEW.accepted=0 AND NEW.acceptance_kind IS NOT NULL)
 OR (NEW.accepted=1 AND NEW.acceptance_kind IS NULL)
 OR (OLD.acceptance_kind IS NOT NULL AND NEW.acceptance_kind IS NOT OLD.acceptance_kind)
 BEGIN SELECT RAISE(ABORT,'initial telephone acceptance kind is immutable'); END;
DROP TRIGGER initial_telephone_acceptance_binding;
CREATE TRIGGER initial_telephone_acceptance_binding BEFORE UPDATE ON initial_telephone_answers
 WHEN NEW.accepted=1 AND NOT (
 (NEW.acceptance_kind='public-session' AND EXISTS(
 SELECT 1 FROM operations a JOIN operation_control_receipts r ON r.operation_record_id=a.record_id
 JOIN initial_telephone_operations n ON n.operation_record_id=NEW.operation_record_id
 JOIN operations p ON p.record_id=n.operation_record_id
 WHERE n.parent_kind='invitation' AND a.record_id=NEW.acceptance_record_id AND a.state='completed' AND a.command_type='session.accept'
 AND a.operation_id='answer_'||n.operation_record_id AND a.session_id=p.session_id AND a.conversation_id=p.conversation_id
 AND a.owner_id=json_extract(n.source_json,'$.target.identity_id')
 AND r.actor_participant_id=n.target_participant_id
 AND r.actor_membership_epoch=json_extract(n.source_json,'$.target.membership_epoch')))
 OR (NEW.acceptance_kind='private-connection' AND NEW.acceptance_record_id=NEW.operation_record_id AND EXISTS(
 SELECT 1 FROM initial_telephone_private_acceptances own JOIN initial_telephone_operations n ON n.operation_record_id=own.operation_record_id
 WHERE own.operation_record_id=NEW.operation_record_id AND n.parent_kind='consultation'
 AND own.consultation_record_id=n.parent_record_id AND own.codec=NEW.codec)))
 BEGIN SELECT RAISE(ABORT,'initial telephone acceptance has no matching typed own receipt'); END;
PRAGMA user_version=106;
