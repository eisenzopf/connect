ALTER TABLE voice_calls ADD COLUMN end_credential_id TEXT REFERENCES credentials(id);
-- Previous binaries did not record who authorized an end request. Reconcile
-- these calls by reading provider status, rather than repeating an effect.
UPDATE voice_calls SET end_requested=2 WHERE end_requested=1;
ALTER TABLE voice_ingress ADD COLUMN scanned_until INTEGER NOT NULL DEFAULT 0;
PRAGMA user_version=6;
