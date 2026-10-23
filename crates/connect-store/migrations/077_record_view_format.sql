-- Fresh schema boundary for the caller-projectable metadata format. Old private
-- canonical records are not silently treated as this public read format.
ALTER TABLE session_record_intents ADD COLUMN format_version INTEGER NOT NULL DEFAULT 2 CHECK(format_version=2);
PRAGMA user_version=77;
