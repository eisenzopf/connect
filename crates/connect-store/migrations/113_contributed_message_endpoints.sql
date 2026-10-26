-- Connector locators select a declaration; routes retain the decoded target.
-- The Store validates decoding and full current authority before this insert.
-- Bind the exact authorized facts as well as the explicit transport profile.
DROP TRIGGER provider_message_endpoint_binding;
CREATE TRIGGER provider_message_endpoint_binding BEFORE INSERT ON provider_message_endpoints
 WHEN NEW.route_id IS NOT json_extract(NEW.policy_json,'$.route_id')
 OR NOT EXISTS(SELECT 1 FROM connector_routes r JOIN endpoints e ON e.id=json_extract(NEW.policy_json,'$.endpoint_id')
 WHERE r.id=NEW.route_id AND r.identity_id=e.identity_id
 AND r.connector=json_extract(NEW.policy_json,'$.connector')
 AND e.transport=json_extract(NEW.policy_json,'$.transport')
 AND e.verification_state='verified'
 AND (r.target=e.address OR
      (e.transport='connector'
       AND instr(e.address,'connector://'||r.connector||'/')=1
       AND r.target=json_extract(NEW.source_json,'$.route.target')
       AND e.address=json_extract(NEW.source_json,'$.target.address'))))
 BEGIN SELECT RAISE(ABORT,'provider message endpoint binding mismatch'); END;
PRAGMA user_version=113;
