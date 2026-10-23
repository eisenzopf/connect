-- Durable canonical bytes and source manifest. Storage ownership is never a
-- grant to see a source; Service retains current authority during projection.
CREATE TABLE session_records (
 uuid TEXT PRIMARY KEY,
 tenant_id TEXT NOT NULL,
 conversation_id TEXT NOT NULL,
 session_id TEXT NOT NULL REFERENCES sessions(id),
 location TEXT NOT NULL UNIQUE,
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
 body BLOB,
 manifest BLOB,
 body_hash TEXT NOT NULL,
 manifest_hash TEXT NOT NULL,
 deleted INTEGER NOT NULL DEFAULT 0 CHECK(deleted IN (0,1)),
 CHECK((deleted=0 AND body IS NOT NULL AND manifest IS NOT NULL)
    OR (deleted=1 AND body IS NULL AND manifest IS NULL)),
 FOREIGN KEY(session_id,conversation_id) REFERENCES sessions(id,conversation_id),
 FOREIGN KEY(conversation_id,tenant_id) REFERENCES conversations(id,tenant_id)
);
CREATE INDEX session_records_scope ON session_records(tenant_id,conversation_id,session_id,uuid);
CREATE TABLE session_record_mutations (
 uuid TEXT NOT NULL REFERENCES session_records(uuid),
 operation_id TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('delete','redact')),
 fingerprint TEXT NOT NULL,
 expected_json TEXT NOT NULL,
 result_json TEXT NOT NULL,
 at TEXT NOT NULL,
 PRIMARY KEY(uuid,operation_id)
);
CREATE TRIGGER session_record_owner_immutable BEFORE UPDATE ON session_records
 WHEN NEW.uuid!=OLD.uuid OR NEW.tenant_id!=OLD.tenant_id
 OR NEW.conversation_id!=OLD.conversation_id OR NEW.session_id!=OLD.session_id
 OR NEW.location!=OLD.location
 BEGIN SELECT RAISE(ABORT,'record ownership is immutable'); END;
PRAGMA user_version=75;
