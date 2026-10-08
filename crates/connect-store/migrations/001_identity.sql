CREATE TABLE installation (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    id TEXT NOT NULL UNIQUE,
    cursor_key TEXT NOT NULL
);
CREATE TABLE identities (
    id TEXT PRIMARY KEY,
    alias TEXT NOT NULL UNIQUE,
    capabilities TEXT NOT NULL
);
CREATE TABLE credentials (
    id TEXT PRIMARY KEY,
    identity_id TEXT NOT NULL REFERENCES identities(id),
    verifier TEXT NOT NULL UNIQUE,
    operations TEXT NOT NULL,
    recipients TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    revoked INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1))
);
PRAGMA user_version = 1;
