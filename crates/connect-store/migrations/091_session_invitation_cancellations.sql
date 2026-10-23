-- Only targets pending at the successful cancellation boundary receive a hint.
CREATE TABLE session_invitation_cancellations (
 source_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 by_participant_id TEXT NOT NULL,
 by_membership_epoch INTEGER NOT NULL CHECK(by_membership_epoch BETWEEN 1 AND 9007199254740991),
 reason_code INTEGER NOT NULL CHECK(reason_code BETWEEN 0 AND 65535),
 reason TEXT NOT NULL CHECK(length(CAST(reason AS BLOB))<=256),
 cancelled_at INTEGER NOT NULL CHECK(cancelled_at>0),
 PRIMARY KEY(source_id,participant_id),
 FOREIGN KEY(source_id,participant_id) REFERENCES session_invitation_targets(source_id,participant_id)
);
CREATE TRIGGER session_invitation_cancellation_binding BEFORE INSERT ON session_invitation_cancellations
 WHEN NOT EXISTS(SELECT 1 FROM session_invitation_targets t JOIN session_invitation_slots slot ON slot.source_id=t.source_id AND slot.participant_id=t.participant_id AND slot.membership_epoch=t.membership_epoch JOIN session_participants p ON p.session_id=slot.session_id AND p.participant_id=slot.participant_id AND p.membership_epoch=slot.membership_epoch AND p.state='invited' WHERE t.source_id=NEW.source_id AND t.participant_id=NEW.participant_id AND t.membership_epoch=NEW.membership_epoch)
 BEGIN SELECT RAISE(ABORT,'cancellation requires the latest pending target'); END;
CREATE TRIGGER session_invitation_cancellation_immutable BEFORE UPDATE ON session_invitation_cancellations
 BEGIN SELECT RAISE(ABORT,'original invitation cancellation is immutable'); END;
CREATE TRIGGER session_invitation_cancellation_reserved BEFORE DELETE ON session_invitation_cancellations
 BEGIN SELECT RAISE(ABORT,'original invitation cancellation remains reserved'); END;
PRAGMA user_version=91;
