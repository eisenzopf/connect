-- Original per-invocation source, captured in the same operation savepoint.
-- Deferred receipt binding is satisfied before the outer transaction commits.
CREATE TABLE session_invitation_sources (
 id TEXT PRIMARY KEY,
 tenant_id TEXT NOT NULL,
 owner_id TEXT NOT NULL REFERENCES identities(id),
 operation_id TEXT NOT NULL,
 conversation_id TEXT NOT NULL,
 session_id TEXT NOT NULL,
 from_participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 from_epoch INTEGER NOT NULL CHECK(from_epoch BETWEEN 1 AND 9007199254740991),
 medium TEXT NOT NULL,
 intent TEXT NOT NULL,
 capabilities_json TEXT NOT NULL CHECK(json_valid(capabilities_json) AND json_type(capabilities_json)='object' AND length(CAST(capabilities_json AS BLOB))<=16384),
 created_at INTEGER NOT NULL CHECK(created_at>0),
 UNIQUE(tenant_id,owner_id,operation_id),
 FOREIGN KEY(tenant_id,owner_id,operation_id) REFERENCES operations(tenant_id,owner_id,operation_id) DEFERRABLE INITIALLY DEFERRED,
 FOREIGN KEY(session_id,conversation_id) REFERENCES sessions(id,conversation_id),
 FOREIGN KEY(conversation_id,tenant_id) REFERENCES conversations(id,tenant_id)
);
CREATE TABLE session_invitation_targets (
 source_id TEXT NOT NULL REFERENCES session_invitation_sources(id),
 participant_id TEXT NOT NULL REFERENCES participants(participant_id),
 identity_id TEXT NOT NULL REFERENCES identities(id),
 membership_epoch INTEGER NOT NULL CHECK(membership_epoch BETWEEN 1 AND 9007199254740991),
 PRIMARY KEY(source_id,participant_id)
);
CREATE TABLE session_invitation_slots (
 session_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL,
 source_id TEXT NOT NULL,
 PRIMARY KEY(session_id,participant_id,membership_epoch),
 FOREIGN KEY(session_id,participant_id,membership_epoch) REFERENCES session_participants(session_id,participant_id,membership_epoch),
 FOREIGN KEY(source_id,participant_id) REFERENCES session_invitation_targets(source_id,participant_id)
);
CREATE TRIGGER session_invitation_source_binding BEFORE INSERT ON session_invitation_sources
 WHEN NOT EXISTS(SELECT 1 FROM participants p JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id
 WHERE p.participant_id=NEW.from_participant_id AND p.identity_id=NEW.owner_id AND p.conversation_id=NEW.conversation_id AND p.tenant_id=NEW.tenant_id
 AND p.membership_epoch=NEW.from_epoch AND p.active=1 AND i.revoked=0)
 BEGIN SELECT RAISE(ABORT,'invitation source requires its current original actor'); END;
CREATE TRIGGER session_invitation_target_binding BEFORE INSERT ON session_invitation_targets
 WHEN NOT EXISTS(SELECT 1 FROM session_invitation_sources s JOIN participants p ON p.conversation_id=s.conversation_id AND p.tenant_id=s.tenant_id
 JOIN identities i ON i.id=p.identity_id AND i.tenant_id=p.tenant_id
 WHERE s.id=NEW.source_id AND p.participant_id=NEW.participant_id AND p.identity_id=NEW.identity_id AND p.membership_epoch=NEW.membership_epoch AND p.active=1 AND i.revoked=0)
 BEGIN SELECT RAISE(ABORT,'invitation target requires its original current epoch'); END;
CREATE TRIGGER session_invitation_source_immutable BEFORE UPDATE ON session_invitation_sources
 BEGIN SELECT RAISE(ABORT,'original invitation source is immutable'); END;
CREATE TRIGGER session_invitation_source_reserved BEFORE DELETE ON session_invitation_sources
 BEGIN SELECT RAISE(ABORT,'original invitation source remains reserved'); END;
CREATE TRIGGER session_invitation_target_immutable BEFORE UPDATE ON session_invitation_targets
 BEGIN SELECT RAISE(ABORT,'original invitation target is immutable'); END;
CREATE TRIGGER session_invitation_target_reserved BEFORE DELETE ON session_invitation_targets
 BEGIN SELECT RAISE(ABORT,'original invitation target remains reserved'); END;
CREATE TRIGGER session_invitation_slot_binding BEFORE INSERT ON session_invitation_slots
 WHEN NOT EXISTS(SELECT 1 FROM session_invitation_sources s JOIN session_invitation_targets t ON t.source_id=s.id
 WHERE s.id=NEW.source_id AND s.session_id=NEW.session_id AND t.participant_id=NEW.participant_id AND t.membership_epoch=NEW.membership_epoch)
 BEGIN SELECT RAISE(ABORT,'invitation slot must name its exact original source'); END;
CREATE TRIGGER session_invitation_slot_update_binding BEFORE UPDATE ON session_invitation_slots
 WHEN NEW.session_id!=OLD.session_id OR NEW.participant_id!=OLD.participant_id OR NEW.membership_epoch!=OLD.membership_epoch
 OR NOT EXISTS(SELECT 1 FROM session_invitation_sources s JOIN session_invitation_targets t ON t.source_id=s.id
 WHERE s.id=NEW.source_id AND s.session_id=NEW.session_id AND t.participant_id=NEW.participant_id AND t.membership_epoch=NEW.membership_epoch)
 BEGIN SELECT RAISE(ABORT,'invitation slot replacement must keep its exact member epoch'); END;
PRAGMA user_version=89;
