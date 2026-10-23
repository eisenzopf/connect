-- Fresh-schema boundary. Prior state requires an explicit backup/new state path.
CREATE TABLE session_control (
 session_id TEXT PRIMARY KEY,
 conversation_id TEXT NOT NULL,
 creator_participant_id TEXT NOT NULL,
 creator_membership_epoch INTEGER NOT NULL,
 intent TEXT NOT NULL CHECK(length(intent) BETWEEN 1 AND 128),
 capabilities_json TEXT NOT NULL CHECK(json_valid(capabilities_json)),
 codec_preferences_json TEXT CHECK(codec_preferences_json IS NULL OR json_valid(codec_preferences_json)),
 FOREIGN KEY(session_id,conversation_id) REFERENCES sessions(id,conversation_id),
 FOREIGN KEY(session_id,creator_participant_id,creator_membership_epoch) REFERENCES session_participants(session_id,participant_id,membership_epoch)
);
CREATE TABLE session_presence_capabilities (
 session_id TEXT NOT NULL,
 participant_id TEXT NOT NULL,
 membership_epoch INTEGER NOT NULL,
 capabilities_json TEXT NOT NULL CHECK(json_valid(capabilities_json)),
 PRIMARY KEY(session_id,participant_id,membership_epoch),
 FOREIGN KEY(session_id,participant_id,membership_epoch) REFERENCES session_participants(session_id,participant_id,membership_epoch)
);
CREATE TABLE main_sessions (
 conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
 session_id TEXT NOT NULL UNIQUE,
 FOREIGN KEY(session_id,conversation_id) REFERENCES sessions(id,conversation_id)
);
PRAGMA user_version=35;
