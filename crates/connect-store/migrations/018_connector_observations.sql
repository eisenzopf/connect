-- Normalized lifecycle and actual reply sources commit with message effects.
CREATE TABLE connector_observations (
 job_id TEXT PRIMARY KEY REFERENCES provider_jobs(id),
 projection_json TEXT NOT NULL,
 receipt_json TEXT NOT NULL,
 reply_committed INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE connector_receipt_events (
 job_id TEXT NOT NULL REFERENCES provider_jobs(id),
 event_id TEXT NOT NULL,
 receipt_json TEXT NOT NULL,
 PRIMARY KEY(job_id,event_id)
);
CREATE TABLE connector_incoming (
 route_id TEXT NOT NULL REFERENCES connector_routes(id),
 mutation_id TEXT NOT NULL,
 message_json TEXT NOT NULL,
 message_id TEXT NOT NULL REFERENCES messages(id),
 PRIMARY KEY(route_id,mutation_id)
);
CREATE UNIQUE INDEX connector_incoming_external ON connector_incoming(route_id,json_extract(message_json,'$.external_id'));
CREATE TABLE connector_voice_observations (
 call_id TEXT PRIMARY KEY REFERENCES voice_calls(id),
 projection_json TEXT NOT NULL
);
PRAGMA user_version=18;
