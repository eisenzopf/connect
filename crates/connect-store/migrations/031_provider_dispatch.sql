-- Single-use provider dispatch custody. A claim is not network admission.
CREATE TABLE provider_dispatch (
 job_id TEXT PRIMARY KEY REFERENCES provider_jobs(id),
 state_identity TEXT NOT NULL,
 runtime_id TEXT,
 claim_token TEXT,
 previous_chat_id TEXT,
 admitted INTEGER NOT NULL DEFAULT 0 CHECK(admitted IN (0,1)),
 CHECK((runtime_id IS NULL)=(claim_token IS NULL)),
 CHECK(admitted=0 OR claim_token IS NOT NULL)
);
CREATE TRIGGER provider_dispatch_immutable BEFORE UPDATE ON provider_dispatch
 WHEN NEW.job_id!=OLD.job_id OR NEW.state_identity!=OLD.state_identity
 OR (OLD.claim_token IS NOT NULL AND (NEW.runtime_id IS NOT OLD.runtime_id OR NEW.claim_token IS NOT OLD.claim_token OR NEW.previous_chat_id IS NOT OLD.previous_chat_id))
 OR NEW.admitted<OLD.admitted
 BEGIN SELECT RAISE(ABORT,'provider dispatch custody is immutable'); END;
CREATE TRIGGER provider_dispatch_retained BEFORE DELETE ON provider_dispatch
 BEGIN SELECT RAISE(ABORT,'provider dispatch custody is retained'); END;
PRAGMA user_version=31;
