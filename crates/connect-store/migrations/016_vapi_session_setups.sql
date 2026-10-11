-- A callback listener is ephemeral; its original task binding is not.
-- NULL marks an older task-created session whose private setup cannot be
-- reconstructed from the database. Require a new leg, never guess its intent.
CREATE TABLE vapi_session_setups (
 session_id TEXT PRIMARY KEY REFERENCES sessions(id),
 nonce TEXT UNIQUE,
 binding_json TEXT
);
INSERT INTO vapi_session_setups(session_id,binding_json)
 SELECT DISTINCT resource_id,NULL FROM task_effect_links
 WHERE kind='session' AND resource_id IN (SELECT id FROM sessions);
PRAGMA user_version=16;
