-- Overlap is retained as private custody before Session creation, rather than
-- prohibited configuration or a per-scanner first-match choice. Account owner
-- identity remains unique even across different configured caller numbers.
DROP INDEX provider_voice_ingress_selection;
CREATE INDEX provider_voice_ingress_selection ON provider_voice_ingress(worker_id,
 json_extract(policy_json,'$.provider'),json_extract(policy_json,'$.route_reference'),
 json_extract(source_json,'$.route_owner_id'),json_extract(policy_json,'$.assistant_id'),
 json_extract(policy_json,'$.phone_number_id'),json_extract(source_json,'$.endpoint.address'));
CREATE TRIGGER provider_voice_ingress_account_owner BEFORE INSERT ON provider_voice_ingress
 WHEN EXISTS(SELECT 1 FROM provider_voice_ingress i
 WHERE json_extract(i.policy_json,'$.provider')=json_extract(NEW.policy_json,'$.provider')
 AND json_extract(i.policy_json,'$.route_reference')=json_extract(NEW.policy_json,'$.route_reference')
 AND json_extract(i.source_json,'$.route_owner_id')!=json_extract(NEW.source_json,'$.route_owner_id'))
 BEGIN SELECT RAISE(ABORT,'provider incoming account retains its original owner'); END;
PRAGMA user_version=74;
