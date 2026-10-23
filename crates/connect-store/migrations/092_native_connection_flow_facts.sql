-- Historical engine observations precede hints. They are not current IO grants
-- or Session Active/joined facts; Session lifecycle projection consumes them.
CREATE TABLE native_connection_flow_facts (
 connection_id TEXT NOT NULL REFERENCES connections(id),
 generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
 runtime_epoch TEXT NOT NULL,
 runtime_connection_id TEXT NOT NULL,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 session_id TEXT NOT NULL REFERENCES sessions(id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch>0),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json) AND length(CAST(fact_json AS BLOB))<=32768),
 PRIMARY KEY(connection_id,generation,runtime_epoch),
 CHECK(json_extract(fact_json,'$.runtime_connection_id')=runtime_connection_id AND json_extract(fact_json,'$.generation')=generation)
);
CREATE TRIGGER native_connection_flow_fact_immutable BEFORE UPDATE ON native_connection_flow_facts
 BEGIN SELECT RAISE(ABORT,'original native flow fact is immutable'); END;
CREATE TRIGGER native_connection_flow_fact_reserved BEFORE DELETE ON native_connection_flow_facts
 BEGIN SELECT RAISE(ABORT,'original native flow fact remains reserved'); END;
PRAGMA user_version=92;
