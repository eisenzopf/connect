CREATE TABLE connector_routes (
 id TEXT PRIMARY KEY,
 identity_id TEXT NOT NULL REFERENCES identities(id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 connector TEXT NOT NULL,
 target TEXT NOT NULL,
 source TEXT,
 enabled INTEGER NOT NULL DEFAULT 1,
 UNIQUE(identity_id, conversation_id)
);
CREATE TABLE provider_jobs (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT NOT NULL UNIQUE,
 route_id TEXT NOT NULL REFERENCES connector_routes(id),
 message_id TEXT NOT NULL REFERENCES messages(id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 state TEXT NOT NULL DEFAULT 'pending',
 provider_id TEXT,
 UNIQUE(route_id, message_id)
);
CREATE INDEX provider_jobs_pending ON provider_jobs(state,sequence);
CREATE TABLE provider_events (id TEXT PRIMARY KEY, digest TEXT NOT NULL);
CREATE TABLE provider_context (route_id TEXT PRIMARY KEY REFERENCES connector_routes(id), chat_id TEXT NOT NULL);
PRAGMA user_version=3;
