-- Fresh-schema Gate 6 increment. No automatic upgrade/reset of prior state.
CREATE TABLE membership_invitations (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT NOT NULL UNIQUE,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 tenant_id TEXT NOT NULL,
 identity_id TEXT NOT NULL,
 inviter_id TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('human','ai','system','external')),
 role TEXT NOT NULL CHECK(length(role) BETWEEN 1 AND 64),
 state TEXT NOT NULL CHECK(state IN ('pending','joined','declined','cancelled','expired')),
 expires_at INTEGER NOT NULL,
 created_at INTEGER NOT NULL,
 joined_epoch INTEGER,
 FOREIGN KEY(identity_id,tenant_id) REFERENCES identities(id,tenant_id),
 FOREIGN KEY(inviter_id,tenant_id) REFERENCES identities(id,tenant_id),
 FOREIGN KEY(conversation_id,tenant_id) REFERENCES conversations(id,tenant_id)
);
CREATE UNIQUE INDEX membership_one_invitation ON membership_invitations(conversation_id,identity_id) WHERE state='pending';
CREATE TABLE audience_resources (
 kind TEXT NOT NULL CHECK(kind IN ('message','artifact','session','call','sip_leg','context_delivery','record','transcript','observation')),
 id TEXT NOT NULL,
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 owner_id TEXT NOT NULL REFERENCES identities(id),
 PRIMARY KEY(kind,id), UNIQUE(kind,id,conversation_id)
);
CREATE TABLE resource_audiences (
 kind TEXT NOT NULL,id TEXT NOT NULL,conversation_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 PRIMARY KEY(kind,id,participant_id,membership_epoch),
 FOREIGN KEY(kind,id,conversation_id) REFERENCES audience_resources(kind,id,conversation_id),
 FOREIGN KEY(participant_id,conversation_id) REFERENCES participants(participant_id,conversation_id)
);
CREATE TABLE history_shares (
 id TEXT PRIMARY KEY,conversation_id TEXT NOT NULL REFERENCES conversations(id),
 owner_id TEXT NOT NULL REFERENCES identities(id),provenance TEXT NOT NULL CHECK(length(provenance) BETWEEN 1 AND 512),
 active INTEGER NOT NULL CHECK(active IN (0,1)),created_at INTEGER NOT NULL,revoked_by TEXT REFERENCES identities(id),
 UNIQUE(id,conversation_id)
);
CREATE TABLE history_share_items (
 sharing_id TEXT NOT NULL,conversation_id TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('message','artifact','session','call','sip_leg','context_delivery','record','transcript','observation')),
 source_id TEXT NOT NULL,participant_id TEXT NOT NULL,membership_epoch INTEGER NOT NULL,
 PRIMARY KEY(sharing_id,kind,source_id,participant_id,membership_epoch),
 FOREIGN KEY(sharing_id,conversation_id) REFERENCES history_shares(id,conversation_id),
 FOREIGN KEY(participant_id,conversation_id) REFERENCES participants(participant_id,conversation_id)
);
ALTER TABLE messages ADD COLUMN audience_mode TEXT NOT NULL DEFAULT 'all' CHECK(audience_mode IN ('all','explicit'));
ALTER TABLE artifacts ADD COLUMN audience_mode TEXT NOT NULL DEFAULT 'all' CHECK(audience_mode IN ('all','explicit'));
ALTER TABLE sessions ADD COLUMN audience_mode TEXT NOT NULL DEFAULT 'participants' CHECK(audience_mode IN ('participants','explicit'));
ALTER TABLE tasks ADD COLUMN owner_membership_epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE context_deliveries ADD COLUMN owner_id TEXT REFERENCES identities(id);
ALTER TABLE provider_jobs ADD COLUMN authority_policy TEXT;
ALTER TABLE provider_context ADD COLUMN authority_policy TEXT;

-- An all-audience decision is frozen once at resource insertion. Future
-- addressed writers use explicit and insert their selected epochs atomically.
CREATE TRIGGER register_message_source AFTER INSERT ON messages BEGIN
 INSERT INTO audience_resources VALUES('message',NEW.id,NEW.conversation_id,NEW.sender);
END;
CREATE TRIGGER freeze_message_audience AFTER INSERT ON messages WHEN NEW.audience_mode='all' BEGIN
 UPDATE messages SET sender_participant_id=(SELECT participant_id FROM participants WHERE conversation_id=NEW.conversation_id AND identity_id=NEW.sender AND active=1),accepted_at=strftime('%s','now') WHERE id=NEW.id;
 INSERT INTO message_audiences SELECT NEW.id,NEW.conversation_id,p.participant_id,p.membership_epoch,CASE WHEN p.identity_id=NEW.sender THEN 'sender' ELSE 'recipient' END
 FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id WHERE p.conversation_id=NEW.conversation_id AND p.active=1 AND i.revoked=0;
END;
CREATE TRIGGER register_artifact_source AFTER INSERT ON artifacts BEGIN
 INSERT INTO audience_resources VALUES('artifact',NEW.id,NEW.conversation_id,NEW.owner_id);
END;
CREATE TRIGGER freeze_artifact_audience AFTER INSERT ON artifacts WHEN NEW.audience_mode='all' BEGIN
 INSERT INTO artifact_audiences SELECT NEW.id,NEW.conversation_id,p.participant_id,p.membership_epoch FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id WHERE p.conversation_id=NEW.conversation_id AND p.active=1 AND i.revoked=0;
END;
CREATE TRIGGER freeze_session_audience AFTER INSERT ON sessions BEGIN
 INSERT INTO audience_resources VALUES('session',NEW.id,NEW.conversation_id,NEW.initiator_id);
 INSERT INTO resource_audiences SELECT 'session',NEW.id,NEW.conversation_id,p.participant_id,p.membership_epoch FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id WHERE NEW.audience_mode='participants' AND p.conversation_id=NEW.conversation_id AND p.identity_id IN (NEW.initiator_id,NEW.participant_id) AND p.active=1 AND i.revoked=0;
END;
CREATE TRIGGER freeze_call_audience AFTER INSERT ON voice_calls BEGIN
 INSERT INTO audience_resources SELECT 'call',NEW.id,NEW.conversation_id,c.identity_id FROM credentials c WHERE c.id=NEW.credential_id;
 INSERT INTO resource_audiences SELECT 'call',NEW.id,NEW.conversation_id,p.participant_id,p.membership_epoch FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id WHERE p.conversation_id=NEW.conversation_id AND p.active=1 AND i.revoked=0;
END;
CREATE TRIGGER freeze_sip_audience AFTER INSERT ON sip_legs BEGIN
 INSERT INTO audience_resources VALUES('sip_leg',NEW.id,NEW.conversation_id,NEW.actor_id);
 INSERT INTO resource_audiences SELECT 'sip_leg',NEW.id,NEW.conversation_id,participant_id,membership_epoch FROM resource_audiences WHERE kind='session' AND id=NEW.session_id;
END;
CREATE TRIGGER freeze_context_audience AFTER INSERT ON context_deliveries BEGIN
 UPDATE context_deliveries SET owner_id=COALESCE(NEW.owner_id,(SELECT initiator_id FROM sessions WHERE id=NEW.session_id)) WHERE id=NEW.id;
 INSERT INTO audience_resources SELECT 'context_delivery',NEW.id,NEW.conversation_id,COALESCE(NEW.owner_id,(SELECT initiator_id FROM sessions WHERE id=NEW.session_id)) WHERE COALESCE(NEW.owner_id,(SELECT initiator_id FROM sessions WHERE id=NEW.session_id)) IS NOT NULL;
 INSERT INTO resource_audiences SELECT 'context_delivery',NEW.id,NEW.conversation_id,p.participant_id,p.membership_epoch FROM participants p WHERE p.conversation_id=NEW.conversation_id AND p.identity_id=COALESCE(NEW.owner_id,(SELECT initiator_id FROM sessions WHERE id=NEW.session_id)) AND p.active=1;
END;
CREATE TRIGGER freeze_journal_epoch AFTER INSERT ON journal BEGIN
 UPDATE journal SET recipient_participant_id=COALESCE(NEW.recipient_participant_id,(SELECT participant_id FROM participants WHERE conversation_id=NEW.conversation_id AND identity_id=NEW.recipient AND active=1)),recipient_epoch=COALESCE(NEW.recipient_epoch,(SELECT membership_epoch FROM participants WHERE conversation_id=NEW.conversation_id AND identity_id=NEW.recipient AND active=1)) WHERE sequence=NEW.sequence;
END;
CREATE TRIGGER freeze_task_epoch AFTER INSERT ON tasks BEGIN
 UPDATE tasks SET owner_membership_epoch=COALESCE((SELECT membership_epoch FROM participants WHERE conversation_id=NEW.conversation_id AND identity_id=NEW.owner_id AND active=1),0) WHERE id=NEW.id;
END;

CREATE VIEW original_content_audiences AS
 SELECT 'message' AS kind,message_id AS id,conversation_id,participant_id,membership_epoch FROM message_audiences
 UNION SELECT 'artifact',artifact_id,conversation_id,participant_id,membership_epoch FROM artifact_audiences
 UNION SELECT kind,id,conversation_id,participant_id,membership_epoch FROM resource_audiences;
CREATE VIEW content_audiences AS
 SELECT * FROM original_content_audiences
 UNION SELECT x.kind,x.source_id,x.conversation_id,x.participant_id,x.membership_epoch FROM history_share_items x JOIN history_shares s ON s.id=x.sharing_id WHERE s.active=1;
-- Compound events must satisfy every referenced source, including when a
-- credential or model recipient has a narrower ceiling than its identity.
CREATE VIEW journal_sources AS
 SELECT j.sequence,r.* FROM journal j JOIN audience_resources r ON r.conversation_id=j.conversation_id AND (
 (r.kind='message' AND r.id=j.message_id) OR
 (r.kind='artifact' AND r.id=json_extract(j.artifact_json,'$.id')) OR
 (r.kind='session' AND r.id=json_extract(j.session_json,'$.id')) OR
 (r.kind='call' AND r.id=json_extract(j.call_json,'$.id')) OR
 (r.kind=json_extract(j.data_json,'$.source_kind') AND r.id=json_extract(j.data_json,'$.source_id')));
CREATE VIEW visible_journal AS
 SELECT j.*,COALESCE(
 (SELECT owner_id FROM audience_resources WHERE kind='message' AND id=j.message_id),
 (SELECT owner_id FROM audience_resources WHERE kind='artifact' AND id=json_extract(j.artifact_json,'$.id')),
 (SELECT owner_id FROM audience_resources WHERE kind='session' AND id=json_extract(j.session_json,'$.id')),
 (SELECT owner_id FROM audience_resources WHERE kind='call' AND id=json_extract(j.call_json,'$.id')),
 (SELECT owner_id FROM audience_resources WHERE kind=json_extract(j.data_json,'$.source_kind') AND id=json_extract(j.data_json,'$.source_id')),
 json_extract(j.data_json,'$.identity_id')) AS source_owner,
 CASE WHEN j.message_id IS NOT NULL THEN 'message' WHEN j.artifact_json IS NOT NULL THEN 'artifact' WHEN j.session_json IS NOT NULL THEN 'session' WHEN j.call_json IS NOT NULL THEN 'call' ELSE json_extract(j.data_json,'$.source_kind') END AS source_kind,
 COALESCE(j.message_id,json_extract(j.artifact_json,'$.id'),json_extract(j.session_json,'$.id'),json_extract(j.call_json,'$.id'),json_extract(j.data_json,'$.source_id')) AS source_id FROM journal j JOIN participants p ON p.participant_id=j.recipient_participant_id AND p.conversation_id=j.conversation_id AND p.membership_epoch=j.recipient_epoch AND p.active=1 AND p.identity_id=j.recipient
 JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id AND i.revoked=0
 WHERE (j.message_id IS NULL OR EXISTS(SELECT 1 FROM content_audiences a WHERE a.kind='message' AND a.id=j.message_id AND a.conversation_id=j.conversation_id AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch))
 AND (j.artifact_json IS NULL OR EXISTS(SELECT 1 FROM content_audiences a WHERE a.kind='artifact' AND a.id=json_extract(j.artifact_json,'$.id') AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch))
 AND (j.session_json IS NULL OR EXISTS(SELECT 1 FROM content_audiences a WHERE a.kind='session' AND a.id=json_extract(j.session_json,'$.id') AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch))
 AND (j.call_json IS NULL OR EXISTS(SELECT 1 FROM content_audiences a WHERE a.kind='call' AND a.id=json_extract(j.call_json,'$.id') AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch))
 AND (json_extract(j.data_json,'$.source_id') IS NULL OR EXISTS(SELECT 1 FROM content_audiences a WHERE a.kind=json_extract(j.data_json,'$.source_kind') AND a.id=json_extract(j.data_json,'$.source_id') AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch))
 AND (source_owner IS NULL OR EXISTS(SELECT 1 FROM identities source_identity WHERE source_identity.id=source_owner AND source_identity.tenant_id=p.tenant_id));
CREATE TRIGGER task_owner_epoch_immutable BEFORE UPDATE ON tasks WHEN OLD.owner_membership_epoch!=0 AND NEW.owner_membership_epoch!=OLD.owner_membership_epoch BEGIN SELECT RAISE(ABORT,'task membership epoch is immutable'); END;
PRAGMA user_version=21;
