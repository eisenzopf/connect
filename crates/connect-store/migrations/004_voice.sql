CREATE TABLE voice_routes (
 id TEXT PRIMARY KEY,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 identity_id TEXT NOT NULL REFERENCES identities(id),
 assistant_id TEXT NOT NULL,
 phone_number_id TEXT NOT NULL,
 destinations TEXT NOT NULL,
 enabled INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE voice_calls (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT NOT NULL UNIQUE,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 route_id TEXT NOT NULL REFERENCES voice_routes(id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 destination TEXT NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending',
 provider_id TEXT,
 control_url TEXT,
 ended_reason TEXT,
 end_requested INTEGER NOT NULL DEFAULT 0,
 next_poll INTEGER NOT NULL DEFAULT 0
);
ALTER TABLE journal ADD COLUMN call_json TEXT;
PRAGMA user_version=4;
