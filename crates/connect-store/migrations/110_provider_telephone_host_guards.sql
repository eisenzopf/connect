-- Fresh supported schema only; older stores retain the explicit version refusal.
-- The typed telephone parent replaced invitation_record_id in schema105. Keep
-- both directions of original-invitation host exclusion on that current shape.
DROP TRIGGER provider_voice_no_second_host;
CREATE TRIGGER provider_voice_no_second_host BEFORE INSERT ON provider_session_voice
 WHEN EXISTS(SELECT 1 FROM initial_telephone_operations i
 WHERE i.parent_kind='invitation' AND i.parent_record_id=NEW.invitation_record_id
 AND i.target_participant_id=NEW.target_participant_id)
 BEGIN SELECT RAISE(ABORT,'Session target already has a telephone host'); END;
CREATE TRIGGER initial_telephone_no_second_host BEFORE INSERT ON initial_telephone_operations
 WHEN NEW.parent_kind='invitation' AND EXISTS(SELECT 1 FROM provider_session_voice p
 WHERE p.invitation_record_id=NEW.parent_record_id
 AND p.target_participant_id=NEW.target_participant_id)
 BEGIN SELECT RAISE(ABORT,'Session target already has a provider host'); END;
PRAGMA user_version=110;
