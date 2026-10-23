-- Capture the exact current audience with the immutable terminal source.
CREATE TABLE session_terminal_audiences (
 session_id TEXT NOT NULL REFERENCES session_terminal_facts(session_id),
 identity_id TEXT NOT NULL REFERENCES identities(id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch>0),
 PRIMARY KEY(session_id,participant_id,membership_epoch)
);
CREATE TRIGGER session_terminal_audience_immutable BEFORE UPDATE ON session_terminal_audiences
 BEGIN SELECT RAISE(ABORT,'terminal audience is immutable'); END;
CREATE TRIGGER session_terminal_audience_reserved BEFORE DELETE ON session_terminal_audiences
 BEGIN SELECT RAISE(ABORT,'terminal audience remains reserved'); END;
CREATE TRIGGER session_terminal_hint AFTER INSERT ON session_terminal_facts BEGIN
 INSERT INTO session_terminal_audiences(session_id,identity_id,participant_id,membership_epoch)
 SELECT NEW.session_id,p.identity_id,p.participant_id,p.membership_epoch
 FROM content_audiences a JOIN participants p ON p.participant_id=a.participant_id
 AND p.membership_epoch=a.membership_epoch AND p.conversation_id=a.conversation_id AND p.active=1
 JOIN identities i ON i.id=p.identity_id AND i.revoked=0
 WHERE a.kind='session' AND a.id=NEW.session_id AND a.conversation_id=NEW.conversation_id;
 INSERT INTO session_lifecycle_hints(kind,fact_id,conversation_id,session_id)
 VALUES('session_ended',NEW.session_id,NEW.conversation_id,NEW.session_id);
END;
PRAGMA user_version=95;
