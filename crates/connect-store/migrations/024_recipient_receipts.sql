-- Fresh Gate 6 receipt storage. No upgrade of existing installations.
DROP TABLE message_receipts;
CREATE TABLE message_receipts (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 message_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 channel TEXT NOT NULL CHECK(channel IN ('native','provider')),
 delivery_id TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('queued','accepted','delivered','read','failed','unknown')),
 provenance TEXT NOT NULL CHECK(provenance IN ('journal','client_reported','provider_observation','provider_state','local_attempt')),
 observed_at INTEGER NOT NULL,
 UNIQUE(message_id,participant_id,membership_epoch,channel,delivery_id,kind),
 FOREIGN KEY(message_id,participant_id,membership_epoch) REFERENCES message_audiences(message_id,participant_id,membership_epoch),
 CHECK(kind!='read' OR (channel='native' AND provenance='client_reported'))
);
CREATE TABLE receipt_preferences (
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 delivery INTEGER NOT NULL CHECK(delivery IN (0,1)),
 read INTEGER NOT NULL CHECK(read IN (0,1)),
 PRIMARY KEY(participant_id,membership_epoch)
);
CREATE TRIGGER queued_native_receipt AFTER INSERT ON message_audiences WHEN NEW.purpose='recipient' BEGIN
 INSERT INTO message_receipts(message_id,participant_id,membership_epoch,channel,delivery_id,kind,provenance,observed_at)
 VALUES(NEW.message_id,NEW.participant_id,NEW.membership_epoch,'native',NEW.participant_id,'queued','journal',strftime('%s','now'));
END;
CREATE TRIGGER queued_provider_receipt AFTER INSERT ON provider_jobs BEGIN
 INSERT INTO message_receipts(message_id,participant_id,membership_epoch,channel,delivery_id,kind,provenance,observed_at)
 SELECT NEW.message_id,a.participant_id,a.membership_epoch,'provider',NEW.id,CASE WHEN NEW.state='unsupported' THEN 'failed' ELSE 'queued' END,'journal',strftime('%s','now')
 FROM connector_routes r JOIN participants p ON p.identity_id=r.identity_id AND p.conversation_id=r.conversation_id AND p.active=1
 JOIN message_audiences a ON a.message_id=NEW.message_id AND a.participant_id=p.participant_id AND a.membership_epoch=p.membership_epoch WHERE r.id=NEW.route_id;
END;
-- Provider callbacks and legacy completion paths share these immutable facts.
-- completed means an answer exists; it does not prove delivery or human reading.
CREATE TRIGGER observed_provider_receipt AFTER UPDATE OF state ON provider_jobs WHEN NEW.state!=OLD.state BEGIN
 INSERT OR IGNORE INTO message_receipts(message_id,participant_id,membership_epoch,channel,delivery_id,kind,provenance,observed_at)
 SELECT message_id,participant_id,membership_epoch,'provider',NEW.id,
 CASE WHEN NEW.state='delivered' THEN 'delivered' WHEN NEW.state IN ('accepted','completed') THEN 'accepted' WHEN NEW.state IN ('failed','cancelled','unsupported') THEN 'failed' WHEN NEW.state IN ('sending','uncertain') THEN 'unknown' ELSE 'queued' END,
 'provider_state',strftime('%s','now')
 FROM message_receipts WHERE delivery_id=NEW.id AND channel='provider' AND kind IN ('queued','failed') ORDER BY sequence LIMIT 1;
END;
CREATE VIEW receipt_facts AS SELECT r.*, CASE kind WHEN 'queued' THEN 0 WHEN 'unknown' THEN 1 WHEN 'accepted' THEN 2 WHEN 'failed' THEN 3 WHEN 'delivered' THEN 4 WHEN 'read' THEN 5 END AS rank FROM message_receipts r;
DROP VIEW visible_journal;
CREATE VIEW receipt_audience_journal AS
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
CREATE VIEW visible_journal AS SELECT j.* FROM receipt_audience_journal j WHERE json_extract(j.data_json,'$.receipt') IS NULL OR EXISTS(
 SELECT 1 FROM participants p JOIN identities i ON i.id=p.identity_id AND i.revoked=0 LEFT JOIN receipt_preferences pref ON pref.participant_id=p.participant_id AND pref.membership_epoch=p.membership_epoch
 WHERE p.participant_id=json_extract(j.data_json,'$.receipt.participant_id') AND p.membership_epoch=json_extract(j.data_json,'$.receipt.membership_epoch') AND p.conversation_id=j.conversation_id AND p.active=1
 AND EXISTS(SELECT 1 FROM message_audiences reader WHERE reader.message_id=json_extract(j.data_json,'$.receipt.message_id') AND reader.participant_id=j.recipient_participant_id AND reader.membership_epoch=j.recipient_epoch)
 AND CASE WHEN json_extract(j.data_json,'$.receipt.state')='read' THEN COALESCE(pref.read,0) ELSE COALESCE(pref.delivery,1) END=1);
PRAGMA user_version=24;
