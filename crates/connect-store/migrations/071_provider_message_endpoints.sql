CREATE TABLE provider_message_endpoints (
 route_id TEXT PRIMARY KEY REFERENCES connector_routes(id),
 policy_json TEXT NOT NULL CHECK(json_valid(policy_json) AND length(policy_json)<=8192),
 source_json TEXT NOT NULL CHECK(json_valid(source_json) AND length(source_json)<=32768)
);
CREATE TRIGGER provider_message_endpoint_binding BEFORE INSERT ON provider_message_endpoints
 WHEN NEW.route_id IS NOT json_extract(NEW.policy_json,'$.route_id')
 OR NOT EXISTS(SELECT 1 FROM connector_routes r JOIN endpoints e ON e.id=json_extract(NEW.policy_json,'$.endpoint_id')
 WHERE r.id=NEW.route_id AND r.identity_id=e.identity_id AND r.target=e.address
 AND r.connector=json_extract(NEW.policy_json,'$.connector') AND e.verification_state='verified')
 BEGIN SELECT RAISE(ABORT,'provider message endpoint binding mismatch'); END;
CREATE TRIGGER provider_message_endpoint_immutable BEFORE UPDATE ON provider_message_endpoints
 BEGIN SELECT RAISE(ABORT,'provider message endpoint binding remains immutable'); END;
CREATE TRIGGER provider_message_endpoint_retained BEFORE DELETE ON provider_message_endpoints
 BEGIN SELECT RAISE(ABORT,'provider message endpoint binding remains reserved'); END;
PRAGMA user_version=71;
