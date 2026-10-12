-- Additive: enrollment history, leased authority projection, and independent opt-out.
CREATE TABLE sms_authorities (
 source TEXT PRIMARY KEY, revision INTEGER NOT NULL, digest TEXT NOT NULL,
 valid_until INTEGER NOT NULL
);
CREATE TABLE sms_grants (
 source TEXT NOT NULL, id TEXT NOT NULL, phone TEXT NOT NULL,
 sender TEXT NOT NULL, conversation_id TEXT NOT NULL, evidence TEXT NOT NULL,
 expires_at INTEGER NOT NULL, active INTEGER NOT NULL,
 PRIMARY KEY(source,id)
);
CREATE INDEX sms_grants_recipient ON sms_grants(phone,sender,conversation_id,active);
CREATE TABLE sms_controls (
 phone TEXT PRIMARY KEY, stopped INTEGER NOT NULL, occurred_at INTEGER NOT NULL,
 event_id TEXT NOT NULL
);
CREATE TABLE sms_control_events (
 id TEXT PRIMARY KEY, phone TEXT NOT NULL, sender TEXT NOT NULL,
 keyword TEXT NOT NULL, occurred_at INTEGER NOT NULL, applied INTEGER NOT NULL
);
ALTER TABLE provider_jobs ADD COLUMN sms_decision TEXT;
ALTER TABLE provider_jobs ADD COLUMN sms_grant_id TEXT;
-- Explicit receipt ordering survives VACUUM and backup/restore, unlike implicit rowid.
ALTER TABLE context_deliveries ADD COLUMN sequence INTEGER NOT NULL DEFAULT 0;
UPDATE context_deliveries SET sequence=rowid;
CREATE UNIQUE INDEX context_delivery_sequence ON context_deliveries(sequence);
CREATE INDEX context_delivery_conversation_sequence ON context_deliveries(conversation_id,sequence);
PRAGMA user_version=10;
