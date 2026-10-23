-- A ready record has its own stable source identity. Session history shares
-- grant the same shared record floor, never private captured Connection epochs.
CREATE TRIGGER register_ready_record_source AFTER UPDATE OF state ON session_record_intents
 WHEN NEW.state='ready' AND OLD.state!='ready'
 BEGIN
 INSERT INTO audience_resources(kind,id,conversation_id,owner_id)
 SELECT 'record',NEW.uuid,NEW.conversation_id,initiator_id FROM sessions WHERE id=NEW.session_id AND conversation_id=NEW.conversation_id;
 INSERT INTO resource_audiences(kind,id,conversation_id,participant_id,membership_epoch)
 SELECT 'record',NEW.uuid,NEW.conversation_id,participant_id,membership_epoch FROM resource_audiences WHERE kind='session' AND id=NEW.session_id AND conversation_id=NEW.conversation_id;
 INSERT OR IGNORE INTO history_share_items(sharing_id,conversation_id,kind,source_id,participant_id,membership_epoch)
 SELECT x.sharing_id,x.conversation_id,'record',NEW.uuid,x.participant_id,x.membership_epoch FROM history_share_items x JOIN history_shares s ON s.id=x.sharing_id AND s.active=1 WHERE x.kind='session' AND x.source_id=NEW.session_id AND x.conversation_id=NEW.conversation_id;
 END;
PRAGMA user_version=81;
