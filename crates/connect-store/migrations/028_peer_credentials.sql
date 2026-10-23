-- Fresh state only. Zero is deliberately unusable as canonical issue metadata;
-- the issuer writes the actual time atomically with each new credential.
ALTER TABLE credentials ADD COLUMN issued_at INTEGER NOT NULL DEFAULT 0 CHECK(issued_at>=0);
CREATE TRIGGER credential_owner_immutable BEFORE UPDATE ON credentials
 WHEN NEW.id!=OLD.id OR NEW.identity_id!=OLD.identity_id OR NEW.verifier!=OLD.verifier
 OR NEW.issued_at!=OLD.issued_at OR (OLD.revoked=1 AND NEW.revoked!=1)
 BEGIN SELECT RAISE(ABORT,'credential ownership and issuance are immutable'); END;
ALTER TABLE devices ADD COLUMN revision INTEGER NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991);
CREATE TRIGGER device_owner_immutable BEFORE UPDATE ON devices
 WHEN NEW.id!=OLD.id OR NEW.identity_id!=OLD.identity_id OR NEW.tenant_id!=OLD.tenant_id
 OR NEW.kind!=OLD.kind OR NEW.platform!=OLD.platform OR (OLD.revoked=1 AND NEW.revoked!=1)
 BEGIN SELECT RAISE(ABORT,'device ownership is immutable'); END;
CREATE TABLE peer_signing_keys (
 key_id TEXT PRIMARY KEY,
 identity_id TEXT NOT NULL REFERENCES identities(id),
 tenant_id TEXT NOT NULL REFERENCES tenants(id),
 device_id TEXT NOT NULL REFERENCES devices(id),
 algorithm TEXT NOT NULL CHECK(algorithm IN ('EdDSA','ES256')),
 public_key BLOB NOT NULL CHECK((algorithm='EdDSA' AND length(public_key)=32) OR (algorithm='ES256' AND length(public_key)=65)),
 not_before INTEGER NOT NULL CHECK(not_before>=0),
 expires_at INTEGER NOT NULL CHECK(expires_at>not_before),
 revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
 revision INTEGER NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991),
 FOREIGN KEY(device_id,identity_id,tenant_id) REFERENCES devices(id,identity_id,tenant_id),
 FOREIGN KEY(identity_id,tenant_id) REFERENCES identities(id,tenant_id)
);
CREATE TABLE credential_signing_keys (
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 key_id TEXT NOT NULL REFERENCES peer_signing_keys(key_id),
 PRIMARY KEY(credential_id,key_id)
);
CREATE TRIGGER peer_key_owner_immutable BEFORE UPDATE ON peer_signing_keys
 WHEN NEW.key_id!=OLD.key_id OR NEW.identity_id!=OLD.identity_id OR NEW.tenant_id!=OLD.tenant_id
 OR NEW.device_id!=OLD.device_id OR NEW.algorithm!=OLD.algorithm OR NEW.public_key!=OLD.public_key
 OR NEW.not_before!=OLD.not_before OR NEW.expires_at!=OLD.expires_at OR (OLD.revoked=1 AND NEW.revoked!=1)
 BEGIN SELECT RAISE(ABORT,'signing key ownership is immutable'); END;
CREATE TRIGGER credential_key_owner_required BEFORE INSERT ON credential_signing_keys
 WHEN NOT EXISTS(SELECT 1 FROM credentials c JOIN identities i ON i.id=c.identity_id JOIN peer_signing_keys k ON k.key_id=NEW.key_id JOIN devices d ON d.id=k.device_id
 WHERE c.id=NEW.credential_id AND c.identity_id=k.identity_id AND i.tenant_id=k.tenant_id AND d.identity_id=k.identity_id AND d.tenant_id=k.tenant_id
 AND c.revoked=0 AND i.revoked=0 AND d.revoked=0 AND k.revoked=0)
 BEGIN SELECT RAISE(ABORT,'credential key binding requires one current owner'); END;
CREATE TRIGGER credential_key_binding_immutable BEFORE UPDATE ON credential_signing_keys
 BEGIN SELECT RAISE(ABORT,'credential key binding is immutable'); END;
PRAGMA user_version=28;
