CREATE TABLE session_participant_facts (
 fact_id TEXT PRIMARY KEY,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 session_id TEXT NOT NULL REFERENCES sessions(id),
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch>0),
 kind TEXT NOT NULL CHECK(kind IN ('session_participant_joined','session_participant_left')),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json) AND length(CAST(fact_json AS BLOB))<=65536),
 audience_json TEXT NOT NULL CHECK(json_valid(audience_json)),
 CHECK(json_extract(fact_json,'$.kind')=kind AND json_extract(fact_json,'$.data.fact_id')=fact_id
 AND json_extract(fact_json,'$.data.session.id')=session_id AND json_extract(fact_json,'$.data.session.conversation_id')=conversation_id)
);
CREATE TRIGGER session_participant_fact_immutable BEFORE UPDATE ON session_participant_facts
 BEGIN SELECT RAISE(ABORT,'original participant fact is immutable'); END;
CREATE TRIGGER session_participant_fact_reserved BEFORE DELETE ON session_participant_facts
 BEGIN SELECT RAISE(ABORT,'original participant fact remains reserved'); END;
CREATE TABLE session_connected_presence (
 session_id TEXT NOT NULL REFERENCES sessions(id),
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('joined','left')),
 fact_id TEXT NOT NULL REFERENCES session_participant_facts(fact_id),
 PRIMARY KEY(session_id,participant_id,membership_epoch)
);
CREATE TABLE session_lifecycle_hints (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 kind TEXT NOT NULL,
 fact_id TEXT NOT NULL,
 conversation_id TEXT NOT NULL,
 session_id TEXT NOT NULL,
 UNIQUE(kind,fact_id)
);
CREATE TRIGGER session_start_hint AFTER INSERT ON session_started_facts BEGIN
 INSERT INTO session_lifecycle_hints(kind,fact_id,conversation_id,session_id)
 VALUES('session_started',NEW.session_id,NEW.conversation_id,NEW.session_id);
END;
CREATE TRIGGER session_participant_hint AFTER INSERT ON session_participant_facts BEGIN
 INSERT INTO session_lifecycle_hints(kind,fact_id,conversation_id,session_id)
 VALUES(NEW.kind,NEW.fact_id,NEW.conversation_id,NEW.session_id);
END;
PRAGMA user_version=94;
