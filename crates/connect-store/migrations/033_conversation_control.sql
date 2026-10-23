-- Fresh canonical conversation fields; candidate state is never converted.
ALTER TABLE conversations ADD COLUMN opened_at INTEGER NOT NULL DEFAULT 0;
ALTER TABLE conversations ADD COLUMN idle_close_secs INTEGER CHECK(idle_close_secs BETWEEN 1 AND 604800);
ALTER TABLE conversations ADD COLUMN metadata_json TEXT CHECK(metadata_json IS NULL OR (json_valid(metadata_json) AND length(CAST(metadata_json AS BLOB))<=16384));
ALTER TABLE conversations ADD COLUMN reason_code INTEGER CHECK(reason_code BETWEEN 0 AND 9007199254740991);
ALTER TABLE conversations ADD COLUMN reason TEXT CHECK(reason IS NULL OR length(CAST(reason AS BLOB))<=512);
CREATE TRIGGER conversation_opened_time AFTER INSERT ON conversations WHEN NEW.opened_at=0
 BEGIN UPDATE conversations SET opened_at=strftime('%s','now') WHERE id=NEW.id; END;
PRAGMA user_version=33;
-- Every trusted writer that closes a conversation produces a complete fact.
CREATE TRIGGER conversation_closed_details AFTER UPDATE OF closed ON conversations WHEN NEW.closed=1 AND OLD.closed=0
 BEGIN UPDATE conversations SET reason_code=COALESCE(reason_code,0),reason=COALESCE(reason,'explicit closure') WHERE id=NEW.id; END;
