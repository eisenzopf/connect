-- Fresh Gate 6 schema boundary. Historical databases are backed up and
-- explicitly reinitialized; this file is not an automatic v0.1 migration.
CREATE TABLE tenants (
 id TEXT PRIMARY KEY,
 epoch INTEGER NOT NULL DEFAULT 1 CHECK(epoch BETWEEN 1 AND 9007199254740991)
);
INSERT INTO tenants(id) VALUES('tnt_default');
ALTER TABLE identities ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'tnt_default' REFERENCES tenants(id);
ALTER TABLE identities ADD COLUMN kind TEXT NOT NULL DEFAULT 'human' CHECK(kind IN ('human','ai','system','external'));
ALTER TABLE identities ADD COLUMN display_name TEXT;
CREATE UNIQUE INDEX identities_tenant ON identities(id,tenant_id);
CREATE TABLE identity_subjects (
 tenant_id TEXT NOT NULL,
 issuer TEXT NOT NULL,
 subject TEXT NOT NULL,
 identity_id TEXT NOT NULL,
 PRIMARY KEY(tenant_id,issuer,subject),
 FOREIGN KEY(identity_id,tenant_id) REFERENCES identities(id,tenant_id)
);
ALTER TABLE conversations ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'tnt_default' REFERENCES tenants(id);
ALTER TABLE conversations ADD COLUMN revision INTEGER NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE conversations ADD COLUMN policy TEXT NOT NULL DEFAULT 'persistent' CHECK(policy IN ('persistent','ephemeral'));
CREATE UNIQUE INDEX conversations_tenant ON conversations(id,tenant_id);

-- Keep one membership authority. Participants are appearances of identities,
-- with IDs distinct from identities and epochs that survive removal/rejoin.
DROP TABLE participants;
CREATE TABLE participants (
 conversation_id TEXT NOT NULL,
 identity_id TEXT NOT NULL,
 participant_id TEXT NOT NULL DEFAULT ('part_' || lower(hex(randomblob(16)))) UNIQUE,
 tenant_id TEXT NOT NULL DEFAULT 'tnt_default',
 kind TEXT NOT NULL DEFAULT 'human' CHECK(kind IN ('human','ai','system','external')),
 role TEXT NOT NULL DEFAULT 'participant' CHECK(length(role) BETWEEN 1 AND 64),
 membership_epoch INTEGER NOT NULL DEFAULT 1 CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1)),
 joined_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
 removed_at INTEGER,
 PRIMARY KEY(conversation_id,identity_id),
 UNIQUE(participant_id,conversation_id,tenant_id),
 UNIQUE(participant_id,conversation_id),
 UNIQUE(participant_id,identity_id,tenant_id),
 FOREIGN KEY(conversation_id,tenant_id) REFERENCES conversations(id,tenant_id),
 FOREIGN KEY(identity_id,tenant_id) REFERENCES identities(id,tenant_id)
);
CREATE TRIGGER participant_owner_immutable BEFORE UPDATE ON participants
 WHEN NEW.participant_id!=OLD.participant_id OR NEW.identity_id!=OLD.identity_id
 OR NEW.tenant_id!=OLD.tenant_id OR NEW.conversation_id!=OLD.conversation_id
 BEGIN SELECT RAISE(ABORT,'membership ownership is immutable'); END;
CREATE TRIGGER participant_epoch_monotonic BEFORE UPDATE ON participants
 WHEN (OLD.active=0 AND NEW.active=1 AND NEW.membership_epoch!=OLD.membership_epoch+1)
 OR ((OLD.active!=0 OR NEW.active!=1) AND NEW.membership_epoch!=OLD.membership_epoch)
 BEGIN SELECT RAISE(ABORT,'invalid membership epoch transition'); END;

CREATE TABLE devices (
 id TEXT PRIMARY KEY,
 identity_id TEXT NOT NULL,
 tenant_id TEXT NOT NULL,
 kind TEXT NOT NULL,
 platform TEXT NOT NULL,
 revoked INTEGER NOT NULL DEFAULT 0 CHECK(revoked IN (0,1)),
 UNIQUE(id,identity_id,tenant_id),
 FOREIGN KEY(identity_id,tenant_id) REFERENCES identities(id,tenant_id)
);
CREATE TABLE endpoints (
 id TEXT PRIMARY KEY,
 identity_id TEXT NOT NULL,
 tenant_id TEXT NOT NULL,
 device_id TEXT NOT NULL,
 transport TEXT NOT NULL,
 address TEXT NOT NULL CHECK(length(address) BETWEEN 1 AND 2048),
 capabilities_json TEXT NOT NULL CHECK(json_valid(capabilities_json)),
 verification_state TEXT NOT NULL DEFAULT 'unverified' CHECK(verification_state IN ('unverified','verified','revoked')),
 verification_evidence TEXT,
 verified_at INTEGER,
 revision INTEGER NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991),
 CHECK(verification_state!='verified' OR (verification_evidence IS NOT NULL AND verified_at IS NOT NULL)),
 UNIQUE(id,identity_id,tenant_id),
 FOREIGN KEY(device_id,identity_id,tenant_id) REFERENCES devices(id,identity_id,tenant_id)
);
CREATE INDEX endpoints_identity ON endpoints(tenant_id,identity_id);

ALTER TABLE messages ADD COLUMN sender_participant_id TEXT REFERENCES participants(participant_id);
ALTER TABLE messages ADD COLUMN content_type TEXT NOT NULL DEFAULT 'text/plain';
ALTER TABLE messages ADD COLUMN body_encoding TEXT NOT NULL DEFAULT 'utf8' CHECK(body_encoding IN ('utf8','base64'));
ALTER TABLE messages ADD COLUMN label TEXT NOT NULL DEFAULT 'connect-messages';
ALTER TABLE messages ADD COLUMN in_reply_to_msg_id TEXT REFERENCES messages(id);
ALTER TABLE messages ADD COLUMN accepted_at INTEGER;
CREATE UNIQUE INDEX messages_scope ON messages(id,conversation_id);
CREATE TABLE message_audiences (
 message_id TEXT NOT NULL REFERENCES messages(id),
 conversation_id TEXT NOT NULL,
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 purpose TEXT NOT NULL CHECK(purpose IN ('sender','recipient')),
 PRIMARY KEY(message_id,participant_id,membership_epoch),
 FOREIGN KEY(message_id,conversation_id) REFERENCES messages(id,conversation_id),
 FOREIGN KEY(participant_id,conversation_id) REFERENCES participants(participant_id,conversation_id)
);
CREATE TABLE message_attachments (
 message_id TEXT NOT NULL REFERENCES messages(id),
 artifact_id TEXT NOT NULL REFERENCES artifacts(id),
 conversation_id TEXT NOT NULL,
 content_type TEXT NOT NULL,
 size_bytes INTEGER NOT NULL CHECK(size_bytes>=0),
 PRIMARY KEY(message_id,artifact_id),
 FOREIGN KEY(message_id,conversation_id) REFERENCES messages(id,conversation_id),
 FOREIGN KEY(artifact_id,conversation_id) REFERENCES artifacts(id,conversation_id)
);
CREATE TABLE message_receipts (
 message_id TEXT NOT NULL REFERENCES messages(id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 kind TEXT NOT NULL CHECK(kind IN ('accepted','delivered','read','failed','unknown')),
 connection_id TEXT,
 provenance_json TEXT NOT NULL CHECK(json_valid(provenance_json)),
 observed_at INTEGER NOT NULL,
 PRIMARY KEY(message_id,participant_id,membership_epoch,kind),
 FOREIGN KEY(message_id,participant_id,membership_epoch) REFERENCES message_audiences(message_id,participant_id,membership_epoch)
);
CREATE UNIQUE INDEX artifacts_scope ON artifacts(id,conversation_id);
CREATE TABLE artifact_audiences (
 artifact_id TEXT NOT NULL REFERENCES artifacts(id),
 conversation_id TEXT NOT NULL,
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 PRIMARY KEY(artifact_id,participant_id,membership_epoch),
 FOREIGN KEY(artifact_id,conversation_id) REFERENCES artifacts(id,conversation_id),
 FOREIGN KEY(participant_id,conversation_id) REFERENCES participants(participant_id,conversation_id)
);

ALTER TABLE sessions ADD COLUMN topology_revision INTEGER NOT NULL DEFAULT 0 CHECK(topology_revision BETWEEN 0 AND 9007199254740991);
CREATE UNIQUE INDEX sessions_conversation ON sessions(id,conversation_id);
CREATE TABLE session_participants (
 session_id TEXT NOT NULL,
 conversation_id TEXT NOT NULL,
 tenant_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 role TEXT NOT NULL CHECK(length(role) BETWEEN 1 AND 64),
 state TEXT NOT NULL CHECK(state IN ('invited','joining','present','left','removed')),
 PRIMARY KEY(session_id,participant_id,membership_epoch),
 FOREIGN KEY(session_id,conversation_id) REFERENCES sessions(id,conversation_id),
 FOREIGN KEY(participant_id,conversation_id,tenant_id) REFERENCES participants(participant_id,conversation_id,tenant_id)
);
CREATE TABLE connections (
 id TEXT PRIMARY KEY,
 session_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL,
 identity_id TEXT NOT NULL,
 tenant_id TEXT NOT NULL,
 endpoint_id TEXT NOT NULL,
 generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
 replaces TEXT,
 state TEXT NOT NULL CHECK(state IN ('preparing','ready','active','held','retiring','retired','interrupted','unknown','failed')),
 revision INTEGER NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991),
 observed_runtime_id TEXT,
 observed_at INTEGER,
 readiness_json TEXT CHECK(readiness_json IS NULL OR json_valid(readiness_json)),
 UNIQUE(id,session_id,participant_id),
 UNIQUE(id,session_id),
 CHECK(replaces IS NULL OR replaces!=id),
 FOREIGN KEY(session_id,participant_id,membership_epoch) REFERENCES session_participants(session_id,participant_id,membership_epoch),
 FOREIGN KEY(endpoint_id,identity_id,tenant_id) REFERENCES endpoints(id,identity_id,tenant_id),
 FOREIGN KEY(participant_id,identity_id,tenant_id) REFERENCES participants(participant_id,identity_id,tenant_id),
 FOREIGN KEY(replaces,session_id,participant_id) REFERENCES connections(id,session_id,participant_id)
);
CREATE TRIGGER connection_lineage_generation BEFORE INSERT ON connections
 WHEN NEW.replaces IS NOT NULL AND NOT EXISTS(
 SELECT 1 FROM connections old WHERE old.id=NEW.replaces AND old.generation<NEW.generation)
 BEGIN SELECT RAISE(ABORT,'invalid connection lineage generation'); END;
CREATE TRIGGER connection_owner_immutable BEFORE UPDATE ON connections
 WHEN NEW.id!=OLD.id OR NEW.session_id!=OLD.session_id OR NEW.participant_id!=OLD.participant_id
 OR NEW.membership_epoch!=OLD.membership_epoch OR NEW.identity_id!=OLD.identity_id
 OR NEW.tenant_id!=OLD.tenant_id OR NEW.endpoint_id!=OLD.endpoint_id
 OR NEW.replaces IS NOT OLD.replaces OR NEW.generation<OLD.generation
 BEGIN SELECT RAISE(ABORT,'connection ownership is immutable'); END;

CREATE TABLE operations (
 record_id TEXT PRIMARY KEY,
 tenant_id TEXT NOT NULL,
 owner_id TEXT NOT NULL,
 operation_id TEXT NOT NULL,
 digest TEXT NOT NULL CHECK(length(digest)=64),
 command_type TEXT NOT NULL,
 conversation_id TEXT REFERENCES conversations(id),
 session_id TEXT REFERENCES sessions(id),
 connection_id TEXT REFERENCES connections(id),
 expected_revision INTEGER CHECK(expected_revision BETWEEN 0 AND 9007199254740991),
 intent_json TEXT CHECK(intent_json IS NULL OR (json_valid(intent_json) AND length(CAST(intent_json AS BLOB))<=1048576)),
 state TEXT NOT NULL CHECK(state IN ('accepted','preparing','ready','committing','completed','cancelled','failed','interrupted','unknown')),
 revision INTEGER NOT NULL DEFAULT 0 CHECK(revision BETWEEN 0 AND 9007199254740991),
 result_json TEXT CHECK(result_json IS NULL OR (json_valid(result_json) AND length(CAST(result_json AS BLOB))<=524288)),
 problem_json TEXT CHECK(problem_json IS NULL OR json_valid(problem_json)),
 tombstoned INTEGER NOT NULL DEFAULT 0 CHECK(tombstoned IN (0,1)),
 worker_generation INTEGER NOT NULL DEFAULT 0 CHECK(worker_generation BETWEEN 0 AND 9007199254740991),
 lease_owner TEXT,
 lease_expires_at INTEGER,
 created_at INTEGER NOT NULL,
 updated_at INTEGER NOT NULL,
 expires_at INTEGER,
 UNIQUE(tenant_id,owner_id,operation_id),
 UNIQUE(record_id,tenant_id),
 FOREIGN KEY(owner_id,tenant_id) REFERENCES identities(id,tenant_id),
 FOREIGN KEY(conversation_id,tenant_id) REFERENCES conversations(id,tenant_id),
 FOREIGN KEY(session_id,conversation_id) REFERENCES sessions(id,conversation_id),
 FOREIGN KEY(connection_id,session_id) REFERENCES connections(id,session_id),
 CHECK(session_id IS NULL OR conversation_id IS NOT NULL),
 CHECK(connection_id IS NULL OR session_id IS NOT NULL),
 CHECK(state='completed' OR result_json IS NULL),
 CHECK(tombstoned=0 OR (intent_json IS NULL AND result_json IS NULL))
);
CREATE INDEX operations_conversation ON operations(conversation_id,created_at,record_id);
CREATE TRIGGER operation_intent_immutable BEFORE UPDATE ON operations
 WHEN NEW.record_id!=OLD.record_id OR NEW.tenant_id!=OLD.tenant_id OR NEW.owner_id!=OLD.owner_id
 OR NEW.operation_id!=OLD.operation_id OR NEW.digest!=OLD.digest OR NEW.command_type!=OLD.command_type
 OR NEW.conversation_id IS NOT OLD.conversation_id OR NEW.session_id IS NOT OLD.session_id
 OR NEW.connection_id IS NOT OLD.connection_id OR NEW.expected_revision IS NOT OLD.expected_revision
 OR (NEW.intent_json IS NOT NULL AND NEW.intent_json IS NOT OLD.intent_json)
 OR NEW.worker_generation<OLD.worker_generation
 BEGIN SELECT RAISE(ABORT,'operation intent is immutable'); END;
CREATE TABLE operation_resources (
 operation_record_id TEXT NOT NULL REFERENCES operations(record_id),
 tenant_id TEXT NOT NULL REFERENCES tenants(id),
 resource_kind TEXT NOT NULL,
 resource_id TEXT NOT NULL,
 fencing_generation INTEGER NOT NULL CHECK(fencing_generation BETWEEN 1 AND 9007199254740991),
 PRIMARY KEY(tenant_id,resource_kind,resource_id),
 FOREIGN KEY(operation_record_id,tenant_id) REFERENCES operations(record_id,tenant_id)
);
CREATE TABLE operation_effects (
 id TEXT PRIMARY KEY,
 operation_record_id TEXT NOT NULL REFERENCES operations(record_id),
 provider TEXT NOT NULL,
 correlation_key TEXT NOT NULL,
 idempotency_mode TEXT NOT NULL CHECK(idempotency_mode IN ('none','provider-key','status-query')),
 state TEXT NOT NULL CHECK(state IN ('prepared','submitting','confirmed','failed','cancelled','unknown')),
 attempt INTEGER NOT NULL DEFAULT 0 CHECK(attempt>=0),
 provider_ref TEXT,
 observation_json TEXT CHECK(observation_json IS NULL OR json_valid(observation_json)),
 UNIQUE(provider,correlation_key)
);
CREATE TABLE operation_observations (
 id TEXT PRIMARY KEY,
 operation_record_id TEXT NOT NULL REFERENCES operations(record_id),
 source TEXT NOT NULL,
 source_event_id TEXT NOT NULL,
 digest TEXT NOT NULL CHECK(length(digest)=64),
 actor_id TEXT REFERENCES identities(id),
 data_json TEXT NOT NULL CHECK(json_valid(data_json)),
 observed_at INTEGER NOT NULL,
 UNIQUE(source,source_event_id)
);
ALTER TABLE journal ADD COLUMN recipient_participant_id TEXT REFERENCES participants(participant_id);
ALTER TABLE journal ADD COLUMN recipient_epoch INTEGER CHECK(recipient_epoch BETWEEN 1 AND 9007199254740991);
ALTER TABLE journal ADD COLUMN resource_revision INTEGER CHECK(resource_revision BETWEEN 0 AND 9007199254740991);
ALTER TABLE journal ADD COLUMN operation_record_id TEXT REFERENCES operations(record_id);
ALTER TABLE journal ADD COLUMN data_json TEXT CHECK(data_json IS NULL OR json_valid(data_json));
PRAGMA user_version=19;
