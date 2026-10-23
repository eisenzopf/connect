-- Trusted installation settings affect future terminal intents only.
CREATE TABLE record_retention_settings (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 policy_json TEXT NOT NULL CHECK(json_valid(policy_json) AND length(policy_json)<=4096)
);
INSERT INTO record_retention_settings VALUES(1,'{"name":"connect-local-until-deleted-v1","metadata":{"kind":"until-deleted"},"transcript":{"kind":"until-deleted"},"recorded_media":{"kind":"until-deleted"},"provider_reference":{"kind":"until-deleted"}}');
CREATE TABLE session_record_retention (
 uuid TEXT PRIMARY KEY REFERENCES session_record_intents(uuid),
 policy_json TEXT NOT NULL CHECK(json_valid(policy_json) AND length(policy_json)<=4096)
);
CREATE TRIGGER freeze_record_retention AFTER INSERT ON session_record_intents
 BEGIN INSERT INTO session_record_retention SELECT NEW.uuid,policy_json FROM record_retention_settings WHERE singleton=1; END;
CREATE TRIGGER frozen_record_retention_immutable BEFORE UPDATE ON session_record_retention
 BEGIN SELECT RAISE(ABORT,'terminal retention policy is immutable'); END;
CREATE TRIGGER frozen_record_retention_retained BEFORE DELETE ON session_record_retention
 BEGIN SELECT RAISE(ABORT,'terminal retention policy remains reserved'); END;
ALTER TABLE session_records ADD COLUMN retention_due INTEGER;
ALTER TABLE session_records ADD COLUMN document_expires_at INTEGER;
CREATE INDEX record_retention_due ON session_records(retention_due,uuid) WHERE deleted=0 AND retention_due IS NOT NULL;
CREATE TABLE record_retention_jobs (
 operation_id TEXT PRIMARY KEY,
 uuid TEXT NOT NULL REFERENCES session_records(uuid),
 expected_revision INTEGER NOT NULL,
 expected_json TEXT NOT NULL CHECK(json_valid(expected_json)),
 cutoff INTEGER NOT NULL,
 decision_json TEXT NOT NULL CHECK(json_valid(decision_json) AND length(decision_json)<=1048576),
 state TEXT NOT NULL CHECK(state IN ('pending','applied','superseded')),
 UNIQUE(uuid,expected_revision)
);
CREATE UNIQUE INDEX one_pending_record_retention ON record_retention_jobs(uuid) WHERE state='pending';
CREATE TRIGGER retention_job_immutable BEFORE UPDATE ON record_retention_jobs
 WHEN NEW.operation_id!=OLD.operation_id OR NEW.uuid!=OLD.uuid OR NEW.expected_revision!=OLD.expected_revision
 OR NEW.expected_json!=OLD.expected_json OR NEW.cutoff!=OLD.cutoff OR NEW.decision_json!=OLD.decision_json
 OR OLD.state!='pending' OR NEW.state NOT IN ('applied','superseded')
 BEGIN SELECT RAISE(ABORT,'retention decision is immutable'); END;
CREATE TRIGGER retention_job_retained BEFORE DELETE ON record_retention_jobs
 BEGIN SELECT RAISE(ABORT,'retention decision remains reserved'); END;
PRAGMA user_version=79;
