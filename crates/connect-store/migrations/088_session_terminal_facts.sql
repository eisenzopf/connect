-- Capture terminal metadata before the original record snapshot can be released.
-- This is a new isolated state version, not conversion of earlier installations.
CREATE TABLE session_terminal_facts (
 session_id TEXT PRIMARY KEY REFERENCES sessions(id),
 conversation_id TEXT NOT NULL REFERENCES conversations(id),
 journal_kind TEXT NOT NULL CHECK(journal_kind IN ('session_control','session_ended')),
 resource_revision INTEGER NOT NULL CHECK(resource_revision BETWEEN 0 AND 9007199254740991),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json) AND length(CAST(fact_json AS BLOB))<=65536),
 CHECK(json_extract(fact_json,'$.session.id')=session_id AND json_extract(fact_json,'$.session.conversation_id')=conversation_id AND json_extract(fact_json,'$.session.state')='ended'),
 CHECK(json_type(fact_json,'$.ended_at')='integer' AND json_extract(fact_json,'$.ended_at')>0),
 CHECK(json_type(fact_json,'$.reason_code')='integer' AND json_extract(fact_json,'$.reason_code') BETWEEN 0 AND 65535),
 CHECK(json_type(fact_json,'$.reason')='text' AND length(CAST(json_extract(fact_json,'$.reason') AS BLOB))<=512),
 CHECK((json_type(fact_json,'$.by_participant')='null' AND json_type(fact_json,'$.by_membership_epoch')='null') OR (json_type(fact_json,'$.by_participant')='text' AND json_type(fact_json,'$.by_membership_epoch')='integer' AND json_extract(fact_json,'$.by_membership_epoch') BETWEEN 1 AND 9007199254740991))
);
CREATE VIEW session_terminal_event_snapshot AS
 SELECT s.id AS session_id,s.conversation_id,s.state_revision AS resource_revision,
 json_object('id',s.id,'conversation_id',s.conversation_id,'state',s.state,'media',json(s.media),
 'intent',sc.intent,'capabilities_offer',json(sc.capabilities_json),'codec_preferences',json(COALESCE(sc.codec_preferences_json,'[]')),
 'is_main',json(CASE WHEN EXISTS(SELECT 1 FROM main_sessions m WHERE m.conversation_id=s.conversation_id AND m.session_id=s.id) THEN 'true' ELSE 'false' END),
 'revision',s.state_revision,'topology_revision',s.topology_revision,'created_at',s.created_at,'updated_at',s.updated_at,'ended_reason',s.ended_reason) AS session_json,
 p.participant_id AS by_participant,p.membership_epoch AS by_membership_epoch,
 COALESCE(json_extract(i.snapshot_json,'$.reason_code'),0) AS reason_code,
 COALESCE(json_extract(i.snapshot_json,'$.reason'),'unspecified') AS reason,
 json_extract(i.snapshot_json,'$.ended_at') AS ended_at
 FROM sessions s LEFT JOIN session_control sc ON sc.session_id=s.id AND sc.conversation_id=s.conversation_id
 JOIN session_record_intents i ON i.session_id=s.id AND i.state='pending'
 LEFT JOIN participants p ON p.conversation_id=s.conversation_id AND p.active=1 AND p.identity_id=(SELECT actor FROM journal_context WHERE singleton=1)
 WHERE s.state='ended';
CREATE TRIGGER freeze_session_terminal_fact AFTER INSERT ON journal
 WHEN (NEW.kind='session_ended' OR (NEW.kind='session_control' AND json_extract(NEW.data_json,'$.session.state')='ended'))
 BEGIN
 INSERT OR IGNORE INTO session_terminal_facts(session_id,conversation_id,journal_kind,resource_revision,fact_json)
 SELECT v.session_id,v.conversation_id,NEW.kind,v.resource_revision,
 json_object('session',json(CASE WHEN NEW.kind='session_control' THEN json_extract(NEW.data_json,'$.session') ELSE v.session_json END),
 'by_participant',v.by_participant,'by_membership_epoch',v.by_membership_epoch,'reason_code',v.reason_code,'reason',v.reason,'ended_at',v.ended_at)
 FROM session_terminal_event_snapshot v WHERE v.conversation_id=NEW.conversation_id AND v.session_id=COALESCE(json_extract(NEW.data_json,'$.session.id'),json_extract(NEW.session_json,'$.id'))
 AND NOT EXISTS(SELECT 1 FROM session_terminal_facts f WHERE f.session_id=v.session_id);
 END;
CREATE TRIGGER session_terminal_fact_immutable BEFORE UPDATE ON session_terminal_facts
 BEGIN SELECT RAISE(ABORT,'original Session terminal fact is immutable'); END;
CREATE TRIGGER session_terminal_fact_reserved BEFORE DELETE ON session_terminal_facts
 BEGIN SELECT RAISE(ABORT,'original Session terminal fact remains reserved'); END;
PRAGMA user_version=88;
