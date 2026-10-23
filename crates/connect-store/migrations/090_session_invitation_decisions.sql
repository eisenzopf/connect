-- A server-owned correlated observation resolves one original target slot.
-- This is not a caller command or an ordinary operation receipt.
CREATE TABLE session_invitation_decisions (
 source_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 decision_json TEXT NOT NULL CHECK(json_valid(decision_json) AND length(CAST(decision_json AS BLOB))<=32768),
 outcome_json TEXT NOT NULL CHECK(json_valid(outcome_json) AND length(CAST(outcome_json AS BLOB))<=65536),
 created_at INTEGER NOT NULL CHECK(created_at>0),
 PRIMARY KEY(source_id,participant_id),
 FOREIGN KEY(source_id,participant_id) REFERENCES session_invitation_targets(source_id,participant_id)
);
CREATE TRIGGER session_invitation_decision_binding BEFORE INSERT ON session_invitation_decisions
 WHEN NOT EXISTS(SELECT 1 FROM session_invitation_targets t WHERE t.source_id=NEW.source_id AND t.participant_id=NEW.participant_id AND t.membership_epoch=NEW.membership_epoch)
 BEGIN SELECT RAISE(ABORT,'invitation decision requires its original target epoch'); END;
CREATE TRIGGER session_invitation_decision_immutable BEFORE UPDATE ON session_invitation_decisions
 BEGIN SELECT RAISE(ABORT,'original invitation decision is immutable'); END;
CREATE TRIGGER session_invitation_decision_reserved BEFORE DELETE ON session_invitation_decisions
 BEGIN SELECT RAISE(ABORT,'original invitation decision remains reserved'); END;
PRAGMA user_version=90;
