-- Fresh schema only. Challenges and client bearers are derived, never stored raw.
CREATE TABLE client_enrollments (
 id TEXT PRIMARY KEY,
 tenant_id TEXT NOT NULL REFERENCES tenants(id),
 issuer_identity_id TEXT NOT NULL,
 issuer_credential_id TEXT NOT NULL REFERENCES credentials(id),
 identity_id TEXT NOT NULL,
 device_id TEXT NOT NULL,
 key_id TEXT NOT NULL UNIQUE,
 endpoint_id TEXT NOT NULL,
 approve_operation_id TEXT NOT NULL,
 complete_operation_id TEXT NOT NULL,
 request_json TEXT NOT NULL CHECK(json_valid(request_json) AND length(CAST(request_json AS BLOB))<=65536),
 intent_digest TEXT NOT NULL CHECK(length(intent_digest)=64),
 challenge_hash TEXT NOT NULL CHECK(length(challenge_hash)=64),
 challenge_expires_at INTEGER NOT NULL,
 credential_expires_at INTEGER NOT NULL,
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','completed','exhausted','expired','revoked')),
 attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts BETWEEN 0 AND 5),
 credential_id TEXT UNIQUE REFERENCES credentials(id),
 issued_at INTEGER,
 UNIQUE(tenant_id,issuer_identity_id,approve_operation_id),
 UNIQUE(tenant_id,issuer_identity_id,complete_operation_id),
 FOREIGN KEY(identity_id,tenant_id) REFERENCES identities(id,tenant_id),
 FOREIGN KEY(issuer_identity_id,tenant_id) REFERENCES identities(id,tenant_id),
 CHECK(approve_operation_id!=complete_operation_id),
 CHECK(challenge_expires_at>0 AND credential_expires_at>=challenge_expires_at),
 CHECK((credential_id IS NULL AND issued_at IS NULL AND state!='completed') OR (credential_id IS NOT NULL AND issued_at>0 AND state IN ('completed','revoked')))
);
CREATE INDEX client_enrollment_pending ON client_enrollments(identity_id,state,challenge_expires_at);
CREATE TRIGGER enrollment_intent_immutable BEFORE UPDATE ON client_enrollments
 WHEN NEW.id!=OLD.id OR NEW.tenant_id!=OLD.tenant_id OR NEW.issuer_identity_id!=OLD.issuer_identity_id
 OR NEW.issuer_credential_id!=OLD.issuer_credential_id OR NEW.identity_id!=OLD.identity_id
 OR NEW.device_id!=OLD.device_id OR NEW.key_id!=OLD.key_id OR NEW.endpoint_id!=OLD.endpoint_id
 OR NEW.approve_operation_id!=OLD.approve_operation_id OR NEW.complete_operation_id!=OLD.complete_operation_id
 OR NEW.request_json!=OLD.request_json OR NEW.intent_digest!=OLD.intent_digest OR NEW.challenge_hash!=OLD.challenge_hash
 OR NEW.challenge_expires_at!=OLD.challenge_expires_at OR NEW.credential_expires_at!=OLD.credential_expires_at
 OR (OLD.credential_id IS NOT NULL AND (NEW.credential_id IS NOT OLD.credential_id OR NEW.issued_at IS NOT OLD.issued_at))
 OR NEW.attempts<OLD.attempts
 OR (OLD.state!='pending' AND NEW.state!=OLD.state AND NEW.state!='revoked')
 BEGIN SELECT RAISE(ABORT,'enrollment intent and terminal state are immutable'); END;
PRAGMA user_version=115;
