-- Exact public mutation authority facts survive erased bytes. They are not
-- future grants: every receipt replay must recheck current authority.
CREATE TABLE session_record_mutation_authority (
 uuid TEXT NOT NULL,
 operation_id TEXT NOT NULL,
 actor_id TEXT NOT NULL REFERENCES identities(id),
 intent_hash TEXT NOT NULL,
 authority_json TEXT NOT NULL CHECK(json_valid(authority_json)),
 PRIMARY KEY(uuid,operation_id),
 FOREIGN KEY(uuid,operation_id) REFERENCES session_record_mutations(uuid,operation_id)
);
CREATE TRIGGER record_mutation_authority_immutable BEFORE UPDATE ON session_record_mutation_authority
 BEGIN SELECT RAISE(ABORT,'original mutation authority is immutable'); END;
CREATE TRIGGER record_mutation_authority_retained BEFORE DELETE ON session_record_mutation_authority
 BEGIN SELECT RAISE(ABORT,'original mutation authority is retained'); END;
PRAGMA user_version=78;
