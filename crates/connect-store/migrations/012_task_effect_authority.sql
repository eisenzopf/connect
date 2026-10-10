-- Preserve delivered schema 11. Effect authority is server-derived metadata,
-- never a bearer or a model-supplied principal. Legacy rows remain read-only.
CREATE TABLE task_effect_authority (
 effect_id TEXT PRIMARY KEY REFERENCES task_effects(id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 grant_id TEXT REFERENCES task_grants(id),
 wire_request_id TEXT NOT NULL UNIQUE,
 result_json TEXT,
 error_json TEXT,
 wait_for_reply INTEGER NOT NULL DEFAULT 0 CHECK(wait_for_reply IN (0,1))
);
CREATE TABLE task_effect_links (
 effect_id TEXT NOT NULL REFERENCES task_effects(id),
 kind TEXT NOT NULL CHECK(kind IN ('message','provider_job','call','call_end','sip_leg','sip_end','session')),
 resource_id TEXT NOT NULL,
 PRIMARY KEY(effect_id,kind,resource_id)
);
CREATE UNIQUE INDEX task_effect_message_owner ON task_effect_links(kind,resource_id)
 WHERE kind IN ('message','provider_job','call','sip_leg');
CREATE INDEX task_effect_question_lookup ON task_effects(json_extract(outcome_json,'$.data.message_id'))
 WHERE json_extract(outcome_json,'$.kind')='accepted';
PRAGMA user_version=12;
