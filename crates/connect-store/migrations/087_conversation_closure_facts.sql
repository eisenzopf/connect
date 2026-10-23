-- Original closure facts are reserved when the authoritative journal is written.
-- Gate6 state versions remain explicit; no automatic prior-schema conversion.
CREATE TABLE conversation_closure_facts (
 conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
 reason_code INTEGER NOT NULL CHECK(reason_code BETWEEN 0 AND 9007199254740991),
 reason TEXT NOT NULL CHECK(length(CAST(reason AS BLOB))<=512),
 closed_at INTEGER NOT NULL CHECK(closed_at>0)
);
CREATE TRIGGER freeze_conversation_closure AFTER INSERT ON journal
 WHEN NEW.kind='conversation_closed'
 BEGIN
 INSERT OR IGNORE INTO conversation_closure_facts(conversation_id,reason_code,reason,closed_at)
 SELECT id,reason_code,reason,closed_at FROM conversations WHERE id=NEW.conversation_id AND closed=1;
 SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM conversation_closure_facts f JOIN conversations c ON c.id=f.conversation_id
 WHERE f.conversation_id=NEW.conversation_id AND c.closed=1 AND f.reason_code IS c.reason_code
 AND f.reason IS c.reason AND f.closed_at IS c.closed_at)
 THEN RAISE(ABORT,'journal closure requires an original committed fact') END;
 END;
CREATE TRIGGER conversation_closure_fact_immutable BEFORE UPDATE ON conversation_closure_facts
 BEGIN SELECT RAISE(ABORT,'original Conversation closure is immutable'); END;
CREATE TRIGGER conversation_closure_fact_reserved BEFORE DELETE ON conversation_closure_facts
 BEGIN SELECT RAISE(ABORT,'original Conversation closure remains reserved'); END;
CREATE TRIGGER conversation_closure_state_immutable BEFORE UPDATE ON conversations
 WHEN EXISTS(SELECT 1 FROM conversation_closure_facts f WHERE f.conversation_id=OLD.id
 AND (NEW.closed!=1 OR NEW.closed_at IS NOT f.closed_at OR NEW.reason_code IS NOT f.reason_code OR NEW.reason IS NOT f.reason))
 BEGIN SELECT RAISE(ABORT,'committed Conversation closure cannot change'); END;
PRAGMA user_version=87;
