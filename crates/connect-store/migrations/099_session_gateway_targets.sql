-- URI resolution is frozen with its original invitation, never repeated on replay.
CREATE TABLE session_invitation_gateway_targets (
 source_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,
 reference TEXT NOT NULL CHECK(length(CAST(reference AS BLOB)) BETWEEN 5 AND 2048),
 binding_json TEXT NOT NULL CHECK(json_valid(binding_json) AND json_type(binding_json)='object' AND length(CAST(binding_json AS BLOB))<=16384),
 PRIMARY KEY(source_id,participant_id),
 FOREIGN KEY(source_id,participant_id) REFERENCES session_invitation_targets(source_id,participant_id)
);
CREATE TRIGGER session_gateway_target_immutable BEFORE UPDATE ON session_invitation_gateway_targets
 BEGIN SELECT RAISE(ABORT,'original gateway target is immutable'); END;
CREATE TRIGGER session_gateway_target_reserved BEFORE DELETE ON session_invitation_gateway_targets
 BEGIN SELECT RAISE(ABORT,'original gateway target remains reserved'); END;
PRAGMA user_version=99;
