-- Optional signed pre-F observations only; no CutBound, native IO or grant.
CREATE TABLE native_replacement_public_video_bridge (
 operation_record_id TEXT NOT NULL REFERENCES native_replacement_public_video_reservations(operation_record_id),
 phase TEXT NOT NULL CHECK(phase IN ('opted-in','feeding','decoded-ready')),
 ordinal INTEGER NOT NULL CHECK((phase='opted-in' AND ordinal=1) OR (phase='feeding' AND ordinal=2) OR (phase='decoded-ready' AND ordinal=3)),
 private_readiness_nonce TEXT,
 created_at INTEGER NOT NULL CHECK(created_at>0),
 phase_json TEXT NOT NULL CHECK(json_valid(phase_json) AND length(phase_json)<=16384),
 PRIMARY KEY(operation_record_id,phase), UNIQUE(operation_record_id,ordinal),
 CHECK((phase='opted-in' AND private_readiness_nonce IS NULL) OR (phase!='opted-in' AND private_readiness_nonce IS NOT NULL))
);
CREATE TRIGGER public_video_bridge_retained_update BEFORE UPDATE ON native_replacement_public_video_bridge BEGIN SELECT RAISE(ABORT,'public video bridge immutable'); END;
CREATE TRIGGER public_video_bridge_retained_delete BEFORE DELETE ON native_replacement_public_video_bridge BEGIN SELECT RAISE(ABORT,'public video bridge retained'); END;
CREATE TRIGGER public_video_bridge_insert BEFORE INSERT ON native_replacement_public_video_bridge
 BEGIN
 SELECT RAISE(ABORT,'public video bridge already exists') WHERE EXISTS(SELECT 1 FROM native_replacement_public_video_bridge WHERE operation_record_id=NEW.operation_record_id AND (phase=NEW.phase OR ordinal=NEW.ordinal));
 SELECT CASE WHEN connect_store_native_phase_ledger_v1('public-video-bridge',NEW.operation_record_id,NULL,'public-video',json_array(NEW.operation_record_id,NEW.phase,NEW.ordinal,NEW.private_readiness_nonce,NEW.created_at,NEW.phase_json),NULL,NULL)=1 THEN 1 ELSE RAISE(ABORT,'public video bridge capture required') END;
 END;
PRAGMA user_version=133;
