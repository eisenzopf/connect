-- Application tasks remain children of persistent conversations. No existing
-- identity, message, receipt, session, artifact or SMS table is rebuilt.
CREATE TABLE tasks (
 id TEXT PRIMARY KEY,
 owner_id TEXT NOT NULL REFERENCES identities(id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 scope_json TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('ready','running','waiting','completed','cancelled','failed','expired')),
 revision INTEGER NOT NULL CHECK(revision >= 0),
 expires_at INTEGER NOT NULL,
 initiating_event_id TEXT REFERENCES journal(id),
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 reason TEXT
);
CREATE INDEX tasks_owner_conversation ON tasks(owner_id,conversation_id,created_at,id);
CREATE INDEX tasks_state_expiry ON tasks(state,expires_at);
CREATE TABLE task_events (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT NOT NULL UNIQUE,
 task_id TEXT NOT NULL REFERENCES tasks(id),
 actor_id TEXT NOT NULL REFERENCES identities(id),
 created_at INTEGER NOT NULL,
 data_json TEXT NOT NULL
);
CREATE INDEX task_events_task_sequence ON task_events(task_id,sequence);
CREATE TABLE task_requests (
 actor_id TEXT NOT NULL REFERENCES identities(id),
 id TEXT NOT NULL,
 task_id TEXT NOT NULL REFERENCES tasks(id),
 digest TEXT NOT NULL,
 result_json TEXT NOT NULL,
 PRIMARY KEY(actor_id,id)
);
CREATE TABLE task_bindings (
 adapter TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('native','provider_chat','provider_call','host_session')),
 external_id TEXT NOT NULL,
 task_id TEXT NOT NULL REFERENCES tasks(id),
 registered_by TEXT NOT NULL REFERENCES credentials(id),
 created_at INTEGER NOT NULL,
 PRIMARY KEY(adapter,kind,external_id)
);
CREATE TABLE task_grants (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES tasks(id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 adapter TEXT NOT NULL,
 binding_kind TEXT NOT NULL,
 external_id TEXT NOT NULL,
 operations_json TEXT NOT NULL,
 expires_at INTEGER NOT NULL,
 nonce TEXT NOT NULL,
 verifier TEXT NOT NULL UNIQUE,
 revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
 issuance_actor TEXT NOT NULL REFERENCES identities(id),
 issuance_request_id TEXT NOT NULL,
 issuance_digest TEXT NOT NULL,
 UNIQUE(issuance_actor,issuance_request_id),
 FOREIGN KEY(adapter,binding_kind,external_id) REFERENCES task_bindings(adapter,kind,external_id)
);
CREATE INDEX task_grants_task_expiry ON task_grants(task_id,expires_at,revoked);
CREATE TABLE task_replies (
 event_id TEXT PRIMARY KEY REFERENCES journal(id),
 task_id TEXT REFERENCES tasks(id),
 reply_to TEXT,
 assigned_by TEXT REFERENCES identities(id),
 consumed INTEGER NOT NULL DEFAULT 0 CHECK(consumed IN (0,1))
);
CREATE INDEX task_replies_task_consumed ON task_replies(task_id,consumed,event_id);
CREATE TABLE task_effects (
 id TEXT PRIMARY KEY,
 task_id TEXT NOT NULL REFERENCES tasks(id),
 step_id TEXT NOT NULL,
 request_id TEXT NOT NULL,
 digest TEXT NOT NULL,
 operation_json TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('prepared','dispatched','succeeded','failed','uncertain','cancelled')),
 outcome_json TEXT,
 reply_to TEXT,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 UNIQUE(task_id,step_id,request_id)
);
CREATE INDEX task_effects_task_state ON task_effects(task_id,state);
CREATE TABLE task_claims (
 task_id TEXT NOT NULL REFERENCES tasks(id),
 step_id TEXT NOT NULL,
 worker_id TEXT NOT NULL,
 revision INTEGER NOT NULL CHECK(revision >= 0),
 expires_at INTEGER NOT NULL,
 PRIMARY KEY(task_id,step_id)
);
PRAGMA user_version=11;
