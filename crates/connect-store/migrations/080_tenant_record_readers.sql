-- Explicit installation administration grants a role; ordinary record_get
-- credentials and Conversation membership do not create one.
CREATE TABLE tenant_record_readers (
 identity_id TEXT PRIMARY KEY REFERENCES identities(id),
 tenant_id TEXT NOT NULL REFERENCES tenants(id),
 role TEXT NOT NULL CHECK(length(role) BETWEEN 1 AND 64),
 private_sources INTEGER NOT NULL CHECK(private_sources IN (0,1)),
 active INTEGER NOT NULL CHECK(active IN (0,1)),
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
 FOREIGN KEY(identity_id,tenant_id) REFERENCES identities(id,tenant_id)
);
CREATE TRIGGER tenant_record_reader_owner_immutable BEFORE UPDATE ON tenant_record_readers
 WHEN NEW.identity_id!=OLD.identity_id OR NEW.tenant_id!=OLD.tenant_id OR NEW.revision!=OLD.revision+1
 BEGIN SELECT RAISE(ABORT,'record role ownership is immutable; changes require next revision'); END;
CREATE TRIGGER tenant_record_reader_retained BEFORE DELETE ON tenant_record_readers
 BEGIN SELECT RAISE(ABORT,'record role revisions remain reserved'); END;
PRAGMA user_version=80;
