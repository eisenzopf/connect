-- Fresh candidate boundary. Durable retention loss is distinct from ACL gaps.
ALTER TABLE journal ADD COLUMN observed_at INTEGER NOT NULL DEFAULT 0;
CREATE TRIGGER journal_observed_time AFTER INSERT ON journal WHEN NEW.observed_at=0 BEGIN
 UPDATE journal SET observed_at=strftime('%s','now') WHERE sequence=NEW.sequence;
END;
CREATE TABLE journal_retention (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
 floor INTEGER NOT NULL DEFAULT 0 CHECK(floor>=0)
);
INSERT INTO journal_retention(singleton) VALUES(1);
CREATE TRIGGER journal_retention_gap AFTER DELETE ON journal BEGIN
 UPDATE journal_retention SET floor=MAX(floor,OLD.sequence) WHERE singleton=1;
END;
PRAGMA user_version=26;
