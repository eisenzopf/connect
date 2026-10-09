CREATE TABLE sessions (
 id TEXT PRIMARY KEY,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 initiator_id TEXT NOT NULL REFERENCES identities(id),
 participant_id TEXT NOT NULL REFERENCES identities(id),
 state TEXT NOT NULL,
 media TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 ended_reason TEXT
);
CREATE INDEX sessions_conversation_created ON sessions(conversation_id, created_at, id);
CREATE TABLE session_bindings (
 session_id TEXT NOT NULL REFERENCES sessions(id),
 participant_id TEXT NOT NULL REFERENCES identities(id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 connection_id TEXT NOT NULL,
 stream_id TEXT,
 state TEXT NOT NULL,
 updated_at INTEGER NOT NULL,
 PRIMARY KEY(session_id, participant_id, connection_id)
);
CREATE TABLE availability (
 identity_id TEXT PRIMARY KEY REFERENCES identities(id),
 state TEXT NOT NULL,
 preferred_route TEXT,
 expires_at INTEGER NOT NULL,
 observed_at INTEGER NOT NULL
);
CREATE TABLE artifacts (
 id TEXT PRIMARY KEY,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 owner_id TEXT NOT NULL REFERENCES identities(id),
 name TEXT NOT NULL,
 media_type TEXT NOT NULL,
 expected_size INTEGER NOT NULL,
 expected_sha256 TEXT NOT NULL,
 content BLOB NOT NULL DEFAULT X'',
 blob_name TEXT UNIQUE,
 state TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 finalized_at INTEGER
);
CREATE INDEX artifacts_conversation_created ON artifacts(conversation_id, created_at, id);
CREATE TABLE context_deliveries (
 id TEXT PRIMARY KEY,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 session_id TEXT REFERENCES sessions(id),
 provider TEXT NOT NULL,
 watermark INTEGER NOT NULL,
 selection_json TEXT NOT NULL,
 state TEXT NOT NULL,
 provider_ref TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
ALTER TABLE journal ADD COLUMN session_json TEXT;
ALTER TABLE journal ADD COLUMN artifact_json TEXT;
PRAGMA user_version=7;
