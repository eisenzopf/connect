ALTER TABLE voice_calls ADD COLUMN direction TEXT NOT NULL DEFAULT 'outbound';
CREATE UNIQUE INDEX voice_provider_id ON voice_calls(provider_id) WHERE provider_id IS NOT NULL;
CREATE TABLE voice_ingress (
 route_id TEXT PRIMARY KEY REFERENCES voice_routes(id),
 phone_number_id TEXT NOT NULL UNIQUE,
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 since INTEGER NOT NULL
);
PRAGMA user_version=5;
