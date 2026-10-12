-- Live media observations retain the exact invocation grant without making
-- either a grant ID or the recorded backing credential into cached authority.
-- Existing owner bindings stay nullable; grant retention clears old metadata.
ALTER TABLE session_bindings ADD COLUMN task_grant_id TEXT REFERENCES task_grants(id) ON DELETE SET NULL;
CREATE INDEX session_bindings_task_grant ON session_bindings(task_grant_id);
PRAGMA user_version=15;
