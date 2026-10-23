-- Original readiness identifies a durable record, never bytes or a read grant.
CREATE TABLE record_ready_facts (
 uuid TEXT PRIMARY KEY REFERENCES session_records(uuid),
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 session_id TEXT NOT NULL REFERENCES sessions(id),
 revision INTEGER NOT NULL CHECK(revision=1)
);
CREATE TRIGGER record_ready_source_binding BEFORE INSERT ON record_ready_facts
 WHEN NOT EXISTS(SELECT 1 FROM session_records r JOIN session_record_intents i ON i.uuid=r.uuid
 WHERE r.uuid=NEW.uuid AND r.conversation_id=NEW.conversation_id AND r.session_id=NEW.session_id
 AND r.revision=NEW.revision AND r.body IS NOT NULL AND i.state='ready')
 BEGIN SELECT RAISE(ABORT,'record readiness has no committed outcome'); END;
CREATE TRIGGER record_ready_fact_immutable BEFORE UPDATE ON record_ready_facts
 BEGIN SELECT RAISE(ABORT,'original record readiness is immutable'); END;
CREATE TRIGGER record_ready_fact_reserved BEFORE DELETE ON record_ready_facts
 BEGIN SELECT RAISE(ABORT,'record readiness remains reserved'); END;
CREATE TABLE record_ready_audiences (
 uuid TEXT NOT NULL REFERENCES record_ready_facts(uuid),
 identity_id TEXT NOT NULL REFERENCES identities(id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch>0),
 PRIMARY KEY(uuid,participant_id,membership_epoch)
);
CREATE TRIGGER record_ready_audience_immutable BEFORE UPDATE ON record_ready_audiences
 BEGIN SELECT RAISE(ABORT,'original record audience is immutable'); END;
CREATE TRIGGER record_ready_audience_reserved BEFORE DELETE ON record_ready_audiences
 BEGIN SELECT RAISE(ABORT,'record audience remains reserved'); END;
CREATE TRIGGER record_ready_hint AFTER INSERT ON record_ready_facts BEGIN
 INSERT INTO record_ready_audiences(uuid,identity_id,participant_id,membership_epoch)
 SELECT NEW.uuid,p.identity_id,p.participant_id,p.membership_epoch
 FROM session_record_audiences a JOIN participants p ON p.participant_id=a.participant_id
 AND p.membership_epoch=a.membership_epoch AND p.active=1
 JOIN identities i ON i.id=p.identity_id AND i.revoked=0
 WHERE a.uuid=NEW.uuid AND a.source_group='session';
 INSERT INTO session_lifecycle_hints(kind,fact_id,conversation_id,session_id)
 VALUES('record_ready',NEW.uuid,NEW.conversation_id,NEW.session_id);
END;
PRAGMA user_version=97;
