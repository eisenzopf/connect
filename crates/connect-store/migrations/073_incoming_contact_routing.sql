-- Fresh pre-alpha construction only. Opening populated schema72 still refuses
-- through the normal version boundary; this script cannot erase retained data.
CREATE TEMP TABLE incoming_contact_construction_guard (empty INTEGER NOT NULL CHECK(empty=0));
INSERT INTO incoming_contact_construction_guard SELECT (SELECT count(*) FROM incoming_contact_resources)+(SELECT count(*) FROM incoming_contact_holds)+(SELECT count(*) FROM incoming_contact_receipts)+(SELECT count(*) FROM incoming_contact_deliveries)+(SELECT count(*) FROM incoming_contact_rejections)+(SELECT count(*) FROM incoming_contact_audit);
DROP TABLE incoming_contact_construction_guard;
DROP TABLE incoming_contact_audit;
DROP TABLE incoming_contact_rejections;
DROP TABLE incoming_contact_deliveries;
DROP TABLE incoming_contact_receipts;
DROP TABLE incoming_contact_holds;
CREATE TABLE incoming_contact_holds (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 id TEXT NOT NULL UNIQUE,
 resource_id TEXT NOT NULL REFERENCES incoming_contact_resources(id),
 payload_digest TEXT NOT NULL,
 payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(payload_json)<=65536),
 candidates_json TEXT NOT NULL CHECK(json_valid(candidates_json) AND length(candidates_json)<=262144 AND json_array_length(candidates_json) BETWEEN 1 AND 64),
 reason TEXT NOT NULL CHECK(reason IN ('ambiguous_route','resource_conflict','historical_overlap','routed','ineligible')),
 created_at INTEGER NOT NULL,
 UNIQUE(resource_id,payload_digest)
);
CREATE TABLE incoming_contact_receipts (
 namespace TEXT NOT NULL,
 event_id TEXT NOT NULL CHECK(length(event_id) BETWEEN 1 AND 256),
 digest TEXT NOT NULL,
 hold_id TEXT NOT NULL REFERENCES incoming_contact_holds(id),
 PRIMARY KEY(namespace,event_id)
);
CREATE TABLE incoming_contact_deliveries (
 resource_id TEXT PRIMARY KEY REFERENCES incoming_contact_resources(id),
 hold_id TEXT UNIQUE REFERENCES incoming_contact_holds(id),
 candidate_id TEXT NOT NULL,
 candidate_json TEXT NOT NULL CHECK(json_valid(candidate_json) AND length(candidate_json)<=32768),
 message_id TEXT REFERENCES messages(id),
 provider_call_id TEXT REFERENCES provider_session_voice(id),
 outcome_json TEXT NOT NULL CHECK(json_valid(outcome_json) AND length(outcome_json)<=32768),
 actor_identity_id TEXT NOT NULL REFERENCES identities(id),
 actor_credential_id TEXT NOT NULL REFERENCES credentials(id),
 operation_id TEXT NOT NULL,
 created_at INTEGER NOT NULL,
 CHECK((message_id IS NOT NULL AND provider_call_id IS NULL) OR (message_id IS NULL AND provider_call_id IS NOT NULL))
);
CREATE TABLE incoming_contact_rejections (
 hold_id TEXT PRIMARY KEY REFERENCES incoming_contact_holds(id),
 actor_identity_id TEXT NOT NULL REFERENCES identities(id),
 actor_credential_id TEXT NOT NULL REFERENCES credentials(id),
 operation_id TEXT NOT NULL,
 reason TEXT NOT NULL CHECK(length(reason) BETWEEN 1 AND 256),
 created_at INTEGER NOT NULL,
 UNIQUE(actor_identity_id,operation_id)
);
CREATE TABLE incoming_contact_audit (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT,
 hold_id TEXT NOT NULL REFERENCES incoming_contact_holds(id),
 event_type TEXT NOT NULL CHECK(event_type IN ('held','resolved','rejected')),
 actor_identity_id TEXT NOT NULL REFERENCES identities(id),
 actor_credential_id TEXT NOT NULL REFERENCES credentials(id),
 operation_id TEXT,
 created_at INTEGER NOT NULL
);
CREATE TRIGGER incoming_contact_hold_fixed BEFORE UPDATE ON incoming_contact_holds BEGIN SELECT RAISE(ABORT,'incoming hold remains immutable'); END;
CREATE TRIGGER incoming_contact_hold_retained BEFORE DELETE ON incoming_contact_holds BEGIN SELECT RAISE(ABORT,'incoming hold remains retained'); END;
CREATE TRIGGER incoming_contact_receipt_fixed BEFORE UPDATE ON incoming_contact_receipts BEGIN SELECT RAISE(ABORT,'incoming receipt remains immutable'); END;
CREATE TRIGGER incoming_contact_receipt_retained BEFORE DELETE ON incoming_contact_receipts BEGIN SELECT RAISE(ABORT,'incoming receipt remains retained'); END;
CREATE TRIGGER incoming_contact_delivery_fixed BEFORE UPDATE ON incoming_contact_deliveries BEGIN SELECT RAISE(ABORT,'incoming delivery remains immutable'); END;
CREATE TRIGGER incoming_contact_delivery_retained BEFORE DELETE ON incoming_contact_deliveries BEGIN SELECT RAISE(ABORT,'incoming delivery remains reserved'); END;
CREATE TRIGGER incoming_contact_rejection_fixed BEFORE UPDATE ON incoming_contact_rejections BEGIN SELECT RAISE(ABORT,'incoming rejection remains immutable'); END;
CREATE TRIGGER incoming_contact_rejection_retained BEFORE DELETE ON incoming_contact_rejections BEGIN SELECT RAISE(ABORT,'incoming rejection remains retained'); END;
CREATE TRIGGER incoming_contact_audit_fixed BEFORE UPDATE ON incoming_contact_audit BEGIN SELECT RAISE(ABORT,'incoming audit remains immutable'); END;
CREATE TRIGGER incoming_contact_audit_retained BEFORE DELETE ON incoming_contact_audit BEGIN SELECT RAISE(ABORT,'incoming audit remains retained'); END;
CREATE TRIGGER incoming_contact_delivery_binding BEFORE INSERT ON incoming_contact_deliveries WHEN NEW.hold_id IS NOT NULL AND (
 NOT EXISTS(SELECT 1 FROM incoming_contact_holds h JOIN incoming_contact_resources r ON r.id=h.resource_id, json_each(h.candidates_json) candidate WHERE h.id=NEW.hold_id AND h.resource_id=NEW.resource_id AND h.reason IN ('ambiguous_route','historical_overlap','routed') AND h.payload_digest=r.payload_digest AND json_extract(candidate.value,'$.view.id')=NEW.candidate_id AND json(candidate.value)=json(NEW.candidate_json))
 OR EXISTS(SELECT 1 FROM incoming_contact_rejections WHERE hold_id=NEW.hold_id)
 OR NOT EXISTS(SELECT 1 FROM incoming_contact_resources r WHERE r.id=NEW.resource_id AND r.owner_id=NEW.actor_identity_id AND EXISTS(SELECT 1 FROM credentials k WHERE k.id=NEW.actor_credential_id AND k.identity_id=r.owner_id) AND (
 (r.kind='sms' AND NEW.message_id IS NOT NULL AND EXISTS(SELECT 1 FROM messages m WHERE m.id=NEW.message_id AND m.conversation_id=json_extract(NEW.candidate_json,'$.view.conversation_id') AND m.sender=json_extract(NEW.candidate_json,'$.identity_id') AND m.body=(SELECT json_extract(h.payload_json,'$.body') FROM incoming_contact_holds h WHERE h.id=NEW.hold_id)))
 OR (r.kind='voice' AND NEW.provider_call_id IS NOT NULL AND EXISTS(SELECT 1 FROM provider_session_voice v WHERE v.id=NEW.provider_call_id AND v.direction='inbound' AND v.provider_id=r.provider_id AND v.ingress_id=json_extract(NEW.candidate_json,'$.view.route_id') AND json_extract(v.source_json,'$.request.conversation_id')=json_extract(NEW.candidate_json,'$.view.conversation_id'))))))
 BEGIN SELECT RAISE(ABORT,'incoming delivery requires its original candidate and unclaimed resource'); END;
CREATE TRIGGER incoming_contact_rejection_binding BEFORE INSERT ON incoming_contact_rejections WHEN EXISTS(SELECT 1 FROM incoming_contact_deliveries WHERE hold_id=NEW.hold_id)
 BEGIN SELECT RAISE(ABORT,'incoming contact already resolved'); END;
PRAGMA user_version=73;
