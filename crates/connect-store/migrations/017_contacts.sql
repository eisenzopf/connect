CREATE TABLE contact_pairings (
 id TEXT PRIMARY KEY, owner TEXT NOT NULL REFERENCES identities(id),
 peer TEXT NOT NULL REFERENCES identities(id), credential TEXT NOT NULL REFERENCES credentials(id),
 challenge_hash TEXT NOT NULL, expires_at INTEGER NOT NULL,
 state TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, verified_at INTEGER,
 peer_credential TEXT REFERENCES credentials(id)
);
CREATE INDEX contact_pairings_owner_peer ON contact_pairings(owner,peer);
CREATE TABLE connector_manifests (name TEXT PRIMARY KEY, manifest TEXT NOT NULL);
PRAGMA user_version=17;
