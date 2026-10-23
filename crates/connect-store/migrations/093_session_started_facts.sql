CREATE TABLE session_started_facts (
 session_id TEXT PRIMARY KEY REFERENCES sessions(id),
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json) AND length(CAST(fact_json AS BLOB))<=65536),
 audience_json TEXT NOT NULL CHECK(json_valid(audience_json)),
 CHECK(json_extract(fact_json,'$.session.id')=session_id AND json_extract(fact_json,'$.session.conversation_id')=conversation_id AND json_extract(fact_json,'$.session.state')='active')
);
CREATE TRIGGER session_started_fact_immutable BEFORE UPDATE ON session_started_facts
 BEGIN SELECT RAISE(ABORT,'original Session start is immutable'); END;
CREATE TRIGGER session_started_fact_reserved BEFORE DELETE ON session_started_facts
 BEGIN SELECT RAISE(ABORT,'original Session start remains reserved'); END;
PRAGMA user_version=93;
