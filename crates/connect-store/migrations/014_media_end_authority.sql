-- Keep each end intent's issuer and result separate from resource state. A
-- superseding request must never inherit a historical task's grant or failure.
CREATE TABLE media_end_intents (
 id TEXT PRIMARY KEY,
 kind TEXT NOT NULL CHECK(kind IN ('call_end','sip_end')),
 resource_id TEXT NOT NULL,
 credential_id TEXT REFERENCES credentials(id),
 wire_request_id TEXT,
 effect_id TEXT UNIQUE REFERENCES task_effects(id),
 state TEXT NOT NULL CHECK(state IN ('pending','claimed','dispatching','blocked','superseded','legacy','fulfilled')),
 current INTEGER NOT NULL CHECK(current IN (0,1)),
 UNIQUE(kind,resource_id,wire_request_id)
);
CREATE UNIQUE INDEX media_end_current ON media_end_intents(kind,resource_id) WHERE current=1;
CREATE TABLE media_start_admissions (
 kind TEXT NOT NULL CHECK(kind IN ('call','sip_leg')),
 resource_id TEXT NOT NULL,
 PRIMARY KEY(kind,resource_id)
);
CREATE INDEX task_effect_media_resource ON task_effects(json_extract(operation_json,'$.op'),json_extract(outcome_json,'$.data.id'));
-- Previously unresolved task ends lack a current-intent binding. Retain them as
-- uncertainty, rather than adopting an arbitrary historical effect as authority.
INSERT INTO media_end_intents
 SELECT 'legacy-effect:'||l.effect_id,l.kind,l.resource_id,NULL,NULL,l.effect_id,'legacy',0
 FROM task_effect_links l JOIN task_effects e ON e.id=l.effect_id
 WHERE l.kind IN ('call_end','sip_end') AND e.state IN ('dispatched','uncertain');
-- Existing ordinary voice ends have an issuer. Any task-end history makes the
-- current source ambiguous; require a fresh explicitly authorized request.
INSERT INTO media_end_intents
 SELECT 'legacy-call:'||v.id,'call_end',v.id,v.end_credential_id,NULL,NULL,
 CASE WHEN v.end_credential_id IS NULL OR EXISTS(SELECT 1 FROM task_effect_links l WHERE l.kind='call_end' AND l.resource_id=v.id) THEN 'legacy' ELSE 'pending' END,1
 FROM voice_calls v WHERE v.end_requested=1;
-- Old SIP ends did not retain their issuing credential. Do not substitute the
-- dialer's credential for the person who requested hangup.
INSERT INTO media_end_intents
 SELECT 'legacy-sip:'||s.id,'sip_end',s.id,NULL,NULL,NULL,'legacy',1
 FROM sip_legs s WHERE s.end_requested=1;
PRAGMA user_version=14;
