-- The terminal Session transaction owns the original, bounded metadata intent.
-- No live Core Session, media or external provider is required to freeze it.
ALTER TABLE sessions ADD COLUMN terminal_reason_code INTEGER CHECK(terminal_reason_code IS NULL OR terminal_reason_code BETWEEN 0 AND 65535);
CREATE TABLE session_record_intents (
 uuid TEXT PRIMARY KEY DEFAULT (lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-8'||substr(lower(hex(randomblob(2))),2)||'-8'||substr(lower(hex(randomblob(2))),2)||'-'||lower(hex(randomblob(6)))),
 session_id TEXT NOT NULL UNIQUE,
 tenant_id TEXT NOT NULL,
 conversation_id TEXT NOT NULL,
 policy TEXT NOT NULL CHECK(policy='connect-session-metadata-v1'),
 state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending','ready')),
 snapshot_json TEXT CHECK(snapshot_json IS NULL OR (json_valid(snapshot_json) AND length(snapshot_json)<=1048576)),
 CHECK((state='pending' AND snapshot_json IS NOT NULL) OR (state='ready' AND snapshot_json IS NULL)),
 FOREIGN KEY(session_id,conversation_id) REFERENCES sessions(id,conversation_id),
 FOREIGN KEY(conversation_id,tenant_id) REFERENCES conversations(id,tenant_id)
);
CREATE TABLE session_record_audiences (
 uuid TEXT NOT NULL REFERENCES session_record_intents(uuid),
 source_group TEXT NOT NULL,
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 PRIMARY KEY(uuid,source_group,participant_id,membership_epoch)
);
CREATE VIEW session_record_terminal_snapshot AS
 SELECT s.id AS session_id,c.tenant_id,s.conversation_id,
 json_object('version',1,'state',s.state,'started_at',s.created_at,
 'ended_at',CAST(strftime('%s','now') AS INTEGER),'reason',substr(s.ended_reason,1,1024),
 'reason_code',s.terminal_reason_code,'reason_truncated',COALESCE(length(s.ended_reason)>1024,0),'media',json(s.media),
 'appearance_count',(SELECT count(*) FROM session_participants p WHERE p.session_id=s.id),
 'appearances',json((SELECT json_group_array(json_object('participant_id',p.participant_id,'membership_epoch',p.membership_epoch,'role',p.role,'state',p.state)) FROM (SELECT * FROM session_participants WHERE session_id=s.id ORDER BY participant_id,membership_epoch LIMIT 256) p)),
 'connection_count',(SELECT count(*) FROM connections x WHERE x.session_id=s.id),
 'connections',json((SELECT json_group_array(json_object('connection_id',x.id,'participant_id',x.participant_id,'membership_epoch',x.membership_epoch,'generation',x.generation,'replaces',x.replaces,'state',x.state)) FROM (SELECT * FROM connections WHERE session_id=s.id ORDER BY id LIMIT 512) x)),
 'audience_count',(SELECT count(*) FROM resource_audiences a WHERE a.kind='session' AND a.id=s.id),
 'audience',json((SELECT json_group_array(json_object('participant_id',a.participant_id,'membership_epoch',a.membership_epoch)) FROM (
 SELECT participant_id,membership_epoch FROM resource_audiences WHERE kind='session' AND id=s.id
 UNION SELECT participant_id,membership_epoch FROM participants p
 WHERE p.conversation_id=s.conversation_id AND p.active=1 AND s.audience_mode='participants'
 AND p.identity_id IN(s.initiator_id,s.participant_id)
 AND NOT EXISTS(SELECT 1 FROM audience_resources r WHERE r.kind='session' AND r.id=s.id)
 ORDER BY participant_id,membership_epoch LIMIT 256) a))) AS snapshot_json
 FROM sessions s JOIN conversations c ON c.id=s.conversation_id;
CREATE TRIGGER freeze_session_record_outcome_update AFTER UPDATE OF state ON sessions
 WHEN NEW.state IN ('ended','cancelled','rejected','failed') AND OLD.state NOT IN ('ended','cancelled','rejected','failed')
 BEGIN
 INSERT INTO session_record_intents(session_id,tenant_id,conversation_id,policy,snapshot_json)
 SELECT session_id,tenant_id,conversation_id,'connect-session-metadata-v1',snapshot_json
 FROM session_record_terminal_snapshot WHERE session_id=NEW.id;
 END;
CREATE TRIGGER freeze_session_record_outcome_insert AFTER INSERT ON sessions
 WHEN NEW.state IN ('ended','cancelled','rejected','failed')
 BEGIN
 INSERT INTO session_record_intents(session_id,tenant_id,conversation_id,policy,snapshot_json)
 SELECT session_id,tenant_id,conversation_id,'connect-session-metadata-v1',snapshot_json
 FROM session_record_terminal_snapshot WHERE session_id=NEW.id;
 END;
CREATE TRIGGER session_record_intent_immutable BEFORE UPDATE ON session_record_intents
 WHEN NEW.uuid!=OLD.uuid OR NEW.session_id!=OLD.session_id OR NEW.tenant_id!=OLD.tenant_id
 OR NEW.conversation_id!=OLD.conversation_id OR NEW.policy!=OLD.policy
 OR NOT(OLD.state='pending' AND NEW.state='ready' AND NEW.snapshot_json IS NULL)
 BEGIN SELECT RAISE(ABORT,'record intent may only release its snapshot after exact storage'); END;
CREATE TRIGGER session_record_intent_retained BEFORE DELETE ON session_record_intents
 BEGIN SELECT RAISE(ABORT,'record outcome identity remains reserved'); END;
CREATE TRIGGER session_record_audience_immutable BEFORE UPDATE ON session_record_audiences
 BEGIN SELECT RAISE(ABORT,'original record source epochs are immutable'); END;
CREATE TRIGGER session_record_audience_retained BEFORE DELETE ON session_record_audiences
 BEGIN SELECT RAISE(ABORT,'original record source epochs remain reserved'); END;
PRAGMA user_version=76;
