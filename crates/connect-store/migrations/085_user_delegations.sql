-- Immutable explicit consent and replayable secret issuance, never a role label.
CREATE TABLE user_delegations (
 credential_id TEXT PRIMARY KEY REFERENCES credentials(id),
 issuer_credential_id TEXT NOT NULL REFERENCES credentials(id),
 user_identity_id TEXT NOT NULL REFERENCES identities(id),
 operation_id TEXT NOT NULL CHECK(length(operation_id) BETWEEN 1 AND 128),
 intent_json TEXT NOT NULL CHECK(json_valid(intent_json) AND length(CAST(intent_json AS BLOB))<=65536),
 created_at INTEGER NOT NULL CHECK(created_at>0),
 UNIQUE(user_identity_id,operation_id),
 CHECK(credential_id!=issuer_credential_id)
);
CREATE INDEX user_delegations_issuer ON user_delegations(issuer_credential_id);
CREATE INDEX user_delegations_user ON user_delegations(user_identity_id);
CREATE TRIGGER user_delegation_immutable BEFORE UPDATE ON user_delegations
 BEGIN SELECT RAISE(ABORT,'user delegation is immutable'); END;
CREATE TRIGGER user_delegation_retained BEFORE DELETE ON user_delegations
 BEGIN SELECT RAISE(ABORT,'user consent remains reserved'); END;
PRAGMA user_version=85;
