CREATE TABLE sip_legs (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT NOT NULL UNIQUE,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 session_id TEXT NOT NULL REFERENCES sessions(id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 actor_id TEXT NOT NULL REFERENCES identities(id),
 destination TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending',
 connection_id TEXT UNIQUE,
 ended_reason TEXT,
 end_requested INTEGER NOT NULL DEFAULT 0,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL
);
CREATE INDEX sip_legs_session_sequence ON sip_legs(session_id, sequence);
PRAGMA user_version=9;
