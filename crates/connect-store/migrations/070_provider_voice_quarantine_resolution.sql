-- Retain the original conflict and a separate positive resolution proof.
CREATE TABLE provider_voice_quarantine_resolutions (
 call_id TEXT PRIMARY KEY REFERENCES provider_session_voice(id),
 quarantine_json TEXT NOT NULL CHECK(json_valid(quarantine_json) AND length(quarantine_json)<=16384),
 provider_id TEXT NOT NULL CHECK(length(provider_id) BETWEEN 1 AND 256),
 proof_json TEXT NOT NULL CHECK(json_valid(proof_json) AND length(proof_json)<=16384),
 resolved_at INTEGER NOT NULL
);
CREATE TRIGGER provider_voice_resolution_binding BEFORE INSERT ON provider_voice_quarantine_resolutions
 WHEN NOT EXISTS(SELECT 1 FROM provider_session_voice p JOIN operation_effects e ON e.id=p.create_effect_id
 WHERE p.id=NEW.call_id AND p.direction='outbound' AND p.provider_id IS NULL
 AND p.quarantine_json=NEW.quarantine_json AND p.closed_at IS NULL AND e.state='unknown'
 AND NEW.provider_id=json_extract(NEW.proof_json,'$.provider_id')
 AND NEW.provider_id=json_extract(NEW.proof_json,'$.observation.external_id')
 AND p.id=json_extract(NEW.proof_json,'$.observation.mutation_id')
 AND json_extract(NEW.proof_json,'$.observation.verification')='Reconciliation'
 AND json_extract(NEW.proof_json,'$.observation.state.Voice') IN ('Starting','Active','Ended','Failed','Uncertain')
 AND NOT EXISTS(SELECT 1 FROM provider_session_voice other WHERE other.id!=p.id
 AND other.provider_id=NEW.provider_id
 AND json_extract(other.policy_json,'$.provider')=json_extract(p.policy_json,'$.provider')
 AND json_extract(other.policy_json,'$.route_reference')=json_extract(p.policy_json,'$.route_reference')
 AND json_extract(other.source_json,'$.route_owner_id')=json_extract(p.source_json,'$.route_owner_id')))
 BEGIN SELECT RAISE(ABORT,'provider quarantine resolution lacks original correlated proof'); END;
CREATE TRIGGER provider_voice_resolution_immutable BEFORE UPDATE ON provider_voice_quarantine_resolutions
 BEGIN SELECT RAISE(ABORT,'provider quarantine resolution remains immutable'); END;
CREATE TRIGGER provider_voice_resolution_retained BEFORE DELETE ON provider_voice_quarantine_resolutions
 BEGIN SELECT RAISE(ABORT,'provider quarantine resolution remains reserved'); END;
CREATE TRIGGER provider_voice_quarantine_no_implicit_binding BEFORE UPDATE OF provider_id ON provider_session_voice
 WHEN OLD.provider_id IS NULL AND NEW.provider_id IS NOT NULL AND NEW.quarantine_json IS NOT NULL
 AND NOT EXISTS(SELECT 1 FROM provider_voice_quarantine_resolutions r WHERE r.call_id=NEW.id
 AND r.provider_id=NEW.provider_id AND r.quarantine_json=NEW.quarantine_json)
 BEGIN SELECT RAISE(ABORT,'provider quarantine requires explicit resolution'); END;
PRAGMA user_version=70;
