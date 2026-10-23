-- The default is frozen at invitation creation. Loss is durable; restart does
-- not renew a deadline, and an old process cannot keep a negotiation alive.
CREATE TABLE session_reconnect_policy (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 grace_seconds INTEGER NOT NULL CHECK(grace_seconds BETWEEN 0 AND 300)
);
INSERT INTO session_reconnect_policy VALUES(1,30);
CREATE TABLE session_reconnect_boundaries (
 session_id TEXT PRIMARY KEY REFERENCES sessions(id),
 grace_seconds INTEGER NOT NULL CHECK(grace_seconds BETWEEN 0 AND 300),
 lost_at_millis INTEGER,
 deadline_millis INTEGER,
 CHECK((lost_at_millis IS NULL AND deadline_millis IS NULL) OR
       (lost_at_millis>0 AND deadline_millis=lost_at_millis+grace_seconds*1000))
);
CREATE TRIGGER session_reconnect_invitation AFTER INSERT ON sessions BEGIN
 INSERT INTO session_reconnect_boundaries(session_id,grace_seconds)
 SELECT NEW.id,grace_seconds FROM session_reconnect_policy WHERE singleton=1;
END;
CREATE TRIGGER session_reconnect_policy_frozen BEFORE UPDATE ON session_reconnect_boundaries
 WHEN NEW.session_id!=OLD.session_id OR NEW.grace_seconds!=OLD.grace_seconds
 BEGIN SELECT RAISE(ABORT,'invitation reconnect policy is frozen'); END;
CREATE TRIGGER session_reconnect_terminal AFTER UPDATE OF state ON sessions
 WHEN NEW.state IN ('ended','cancelled','rejected','failed') BEGIN
 UPDATE session_reconnect_boundaries SET lost_at_millis=NULL,deadline_millis=NULL WHERE session_id=NEW.id;
END;
CREATE TABLE native_offer_runtime_admissions (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_offer_operations(operation_record_id),
 runtime_epoch TEXT NOT NULL
);
CREATE TRIGGER native_offer_runtime_immutable BEFORE UPDATE ON native_offer_runtime_admissions
 BEGIN SELECT RAISE(ABORT,'negotiation runtime is immutable'); END;
CREATE TRIGGER native_offer_runtime_reserved BEFORE DELETE ON native_offer_runtime_admissions
 BEGIN SELECT RAISE(ABORT,'negotiation runtime remains reserved'); END;
PRAGMA user_version=96;
