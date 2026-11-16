-- Fresh schema only. These append-only observations authorize no media IO.
CREATE TABLE native_replacement_public_video_reservations (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_replacements(operation_record_id),
 parent_reservation_id TEXT NOT NULL,
 runtime_epoch TEXT NOT NULL,
 plan_digest TEXT NOT NULL,
 candidate_connection_id TEXT NOT NULL,
 candidate_runtime_id TEXT NOT NULL,
 target_generation INTEGER NOT NULL CHECK(target_generation BETWEEN 2 AND 9007199254740991),
 expires_at INTEGER NOT NULL,
 reservation_id TEXT NOT NULL UNIQUE,
 nonce TEXT NOT NULL UNIQUE,
 intent_json TEXT NOT NULL CHECK(json_valid(intent_json) AND length(intent_json)<=4096)
);
CREATE TABLE native_replacement_public_video_phases (
 operation_record_id TEXT NOT NULL REFERENCES native_replacement_public_video_reservations(operation_record_id),
 phase TEXT NOT NULL CHECK(phase IN ('negotiated-never-fed','cut-bound-post-f','decoded-ready','activated','discarded')),
 ordinal INTEGER NOT NULL CHECK(ordinal BETWEEN 1 AND 5),
 private_readiness_nonce TEXT NOT NULL,
 phase_json TEXT NOT NULL CHECK(json_valid(phase_json) AND length(phase_json)<=16384),
 PRIMARY KEY(operation_record_id,phase), UNIQUE(operation_record_id,ordinal)
);
CREATE TABLE native_replacement_public_video_terminals (
 operation_record_id TEXT PRIMARY KEY REFERENCES native_replacement_public_video_reservations(operation_record_id),
 reason TEXT NOT NULL CHECK(reason IN ('cancelled','expired','refused','owner-retired','binding-changed','cut-unavailable'))
);
CREATE TRIGGER public_video_reservation_retained_update BEFORE UPDATE ON native_replacement_public_video_reservations BEGIN SELECT RAISE(ABORT,'public video reservation immutable'); END;
CREATE TRIGGER public_video_reservation_retained_delete BEFORE DELETE ON native_replacement_public_video_reservations BEGIN SELECT RAISE(ABORT,'public video reservation retained'); END;
CREATE TRIGGER public_video_reservation_insert BEFORE INSERT ON native_replacement_public_video_reservations
 BEGIN
 SELECT RAISE(ABORT,'public video reservation already exists') WHERE EXISTS(SELECT 1 FROM native_replacement_public_video_reservations WHERE operation_record_id=NEW.operation_record_id OR reservation_id=NEW.reservation_id OR nonce=NEW.nonce);
 SELECT CASE WHEN connect_store_native_phase_ledger_v1('public-video-reservation',NEW.operation_record_id,NULL,'public-video',json_array(NEW.operation_record_id,NEW.parent_reservation_id,NEW.runtime_epoch,NEW.plan_digest,NEW.candidate_connection_id,NEW.candidate_runtime_id,NEW.target_generation,NEW.expires_at,NEW.reservation_id,NEW.nonce,NEW.intent_json),NULL,NULL)=1 THEN 1 ELSE RAISE(ABORT,'public video reservation capture required') END;
 END;
CREATE TRIGGER public_video_phase_retained_update BEFORE UPDATE ON native_replacement_public_video_phases BEGIN SELECT RAISE(ABORT,'public video phase immutable'); END;
CREATE TRIGGER public_video_phase_retained_delete BEFORE DELETE ON native_replacement_public_video_phases BEGIN SELECT RAISE(ABORT,'public video phase retained'); END;
CREATE TRIGGER public_video_phase_insert BEFORE INSERT ON native_replacement_public_video_phases
 BEGIN
 SELECT RAISE(ABORT,'public video phase already exists') WHERE EXISTS(SELECT 1 FROM native_replacement_public_video_phases WHERE operation_record_id=NEW.operation_record_id AND (phase=NEW.phase OR ordinal=NEW.ordinal));
 SELECT CASE WHEN connect_store_native_phase_ledger_v1('public-video-phase',NEW.operation_record_id,NULL,'public-video',json_array(NEW.operation_record_id,NEW.phase,NEW.ordinal,NEW.private_readiness_nonce,NEW.phase_json),NULL,NULL)=1 THEN 1 ELSE RAISE(ABORT,'public video phase capture required') END;
 END;
CREATE TRIGGER public_video_terminal_retained_update BEFORE UPDATE ON native_replacement_public_video_terminals BEGIN SELECT RAISE(ABORT,'public video terminal immutable'); END;
CREATE TRIGGER public_video_terminal_retained_delete BEFORE DELETE ON native_replacement_public_video_terminals BEGIN SELECT RAISE(ABORT,'public video terminal retained'); END;
CREATE TRIGGER public_video_terminal_insert BEFORE INSERT ON native_replacement_public_video_terminals
 BEGIN
 SELECT RAISE(ABORT,'public video terminal already exists') WHERE EXISTS(SELECT 1 FROM native_replacement_public_video_terminals WHERE operation_record_id=NEW.operation_record_id);
 SELECT CASE WHEN connect_store_native_phase_ledger_v1('public-video-terminal',NEW.operation_record_id,NULL,'public-video',json_array(NEW.operation_record_id,NEW.reason),NULL,NULL)=1 THEN 1 ELSE RAISE(ABORT,'public video terminal capture required') END;
 END;
PRAGMA user_version=132;
