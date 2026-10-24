-- Fresh private SIP candidate metadata precedes allocation. No call, native
-- signing identity, ready observation or IO grant is created by this reservation.
CREATE TABLE sip_consultation_reservations (
 connection_id TEXT PRIMARY KEY REFERENCES connections(id),
 operation_record_id TEXT NOT NULL UNIQUE REFERENCES operations(record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 original_digest TEXT NOT NULL,
 runtime_epoch TEXT NOT NULL,
 primary_connection_id TEXT NOT NULL REFERENCES connections(id),
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 endpoint_revision INTEGER NOT NULL,
 device_id TEXT NOT NULL REFERENCES devices(id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch>0),
 audience_json TEXT NOT NULL CHECK(json_valid(audience_json) AND json_type(audience_json)='array' AND json_array_length(audience_json)=2),
 policy_json TEXT NOT NULL CHECK(json_valid(policy_json) AND length(CAST(policy_json AS BLOB))<=4096),
 CHECK(connection_id!=primary_connection_id)
);
CREATE TRIGGER sip_consultation_reservation_binding BEFORE INSERT ON sip_consultation_reservations
 WHEN NOT EXISTS(
 SELECT 1 FROM operations o JOIN credentials k ON k.id=NEW.credential_id AND k.identity_id=o.owner_id AND k.revoked=0
 JOIN connections p ON p.id=NEW.primary_connection_id AND p.id=o.connection_id AND p.session_id=o.session_id
 JOIN connections c ON c.id=NEW.connection_id AND c.session_id=o.session_id AND c.endpoint_id=NEW.endpoint_id
 JOIN endpoints e ON e.id=NEW.endpoint_id AND e.revision=NEW.endpoint_revision AND e.device_id=NEW.device_id AND e.identity_id=c.identity_id AND e.verification_state='verified' AND e.transport IN ('sip','telephone')
 JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.revoked=0
 JOIN participants m ON m.participant_id=NEW.participant_id AND m.participant_id=c.participant_id AND m.membership_epoch=NEW.membership_epoch AND m.membership_epoch=c.membership_epoch AND m.identity_id=e.identity_id AND m.active=1
 JOIN operation_members original ON original.operation_record_id=o.record_id AND original.participant_id=m.participant_id AND original.membership_epoch=m.membership_epoch
 JOIN credentials endpoint ON endpoint.id=json_extract(NEW.policy_json,'$.endpoint_credential_id') AND endpoint.identity_id=m.identity_id AND endpoint.revoked=0
 JOIN credentials route ON route.id=json_extract(NEW.policy_json,'$.route_credential_id') AND route.identity_id=e.route_owner_id AND route.revoked=0
 WHERE o.record_id=NEW.operation_record_id AND o.command_type='session.consult' AND o.state='accepted' AND o.digest=NEW.original_digest AND o.cancel_requested=0
 AND o.lease_owner IS NULL AND NOT EXISTS(SELECT 1 FROM operation_effects f WHERE f.operation_record_id=o.record_id)
 AND c.generation=1 AND c.state='preparing' AND c.observed_runtime_id IS NULL AND c.readiness_json IS NULL
 AND p.participant_id!=c.participant_id AND json_extract(NEW.policy_json,'$.endpoint_id')=e.id
 AND json_extract(NEW.policy_json,'$.route_reference')=e.secret_ref
 AND NOT EXISTS(SELECT 1 FROM native_consultation_legs n WHERE n.operation_record_id=o.record_id))
 BEGIN SELECT RAISE(ABORT,'private SIP reservation binding mismatch'); END;
CREATE TRIGGER sip_consultation_reservation_immutable BEFORE UPDATE ON sip_consultation_reservations
 BEGIN SELECT RAISE(ABORT,'private SIP reservation is immutable'); END;
CREATE TRIGGER sip_consultation_reservation_retained BEFORE DELETE ON sip_consultation_reservations
 BEGIN SELECT RAISE(ABORT,'private SIP reservation remains reserved'); END;
CREATE TRIGGER native_consultation_no_sip_candidate BEFORE INSERT ON native_consultation_legs
 WHEN EXISTS(SELECT 1 FROM sip_consultation_reservations s WHERE s.operation_record_id=NEW.operation_record_id)
 BEGIN SELECT RAISE(ABORT,'consultation already has its SIP candidate'); END;
CREATE VIEW consultation_candidate_bindings AS
 SELECT connection_id,operation_record_id,credential_id,original_digest,runtime_epoch,primary_connection_id,endpoint_id,endpoint_revision,device_id,audience_json,'native' AS candidate_kind FROM native_consultation_legs
 UNION ALL
 SELECT connection_id,operation_record_id,credential_id,original_digest,runtime_epoch,primary_connection_id,endpoint_id,endpoint_revision,device_id,audience_json,'sip' FROM sip_consultation_reservations;
DROP VIEW native_private_content_floors;
CREATE VIEW native_private_content_floors AS
 SELECT connection_id,primary_connection_id AS partner_connection_id,operation_record_id,audience_json,1 AS is_candidate FROM consultation_candidate_bindings
 UNION ALL
 SELECT l.primary_connection_id,l.connection_id,l.operation_record_id,l.audience_json,0
 FROM consultation_candidate_bindings l JOIN native_consultation_attempts n ON n.operation_record_id=l.operation_record_id
 JOIN operations o ON o.record_id=l.operation_record_id JOIN operation_effects e ON e.id=n.effect_id
 WHERE o.state IN ('preparing','ready','committing','completed','unknown','interrupted')
 AND e.state IN ('prepared','submitting','confirmed','unknown')
 AND NOT EXISTS(SELECT 1 FROM native_consultation_returns r JOIN operation_effects f ON f.id=r.effect_id WHERE r.consultation_record_id=l.operation_record_id AND r.applied_json IS NOT NULL AND f.state='confirmed')
 AND NOT EXISTS(SELECT 1 FROM native_private_lifetimes v WHERE v.operation_record_id=l.operation_record_id AND v.restored_at IS NOT NULL);
PRAGMA user_version=104;
