-- Captured resource descriptions outlive transport readiness. These records
-- are read metadata only; no media admission or physical route uses this table.
CREATE TABLE native_stream_snapshots (
 connection_id TEXT PRIMARY KEY REFERENCES connections(id),
 generation INTEGER NOT NULL CHECK(generation BETWEEN 1 AND 9007199254740991),
 stream_revision INTEGER NOT NULL CHECK(stream_revision BETWEEN 1 AND 9007199254740991),
 available INTEGER NOT NULL CHECK(available IN (0,1)),
 projection_json TEXT NOT NULL CHECK(json_valid(projection_json)
  AND length(CAST(projection_json AS BLOB))<=32768
  AND json_type(projection_json,'$.stream_metadata')='array'
  AND json_array_length(projection_json,'$.stream_metadata')<=64
  AND json_extract(projection_json,'$.stream_metadata_generation')=generation
  AND json_extract(projection_json,'$.stream_metadata_revision')=stream_revision
  AND json_type(projection_json,'$.stream_metadata_counter')='integer'
  AND json_extract(projection_json,'$.stream_metadata_counter') BETWEEN 1 AND 9007199254740991)
);
CREATE TRIGGER native_stream_snapshot_monotonic BEFORE UPDATE ON native_stream_snapshots
 WHEN NEW.connection_id!=OLD.connection_id OR NEW.generation<OLD.generation
 OR (NEW.generation=OLD.generation AND NEW.stream_revision<OLD.stream_revision)
 OR json_extract(NEW.projection_json,'$.stream_metadata_counter')<json_extract(OLD.projection_json,'$.stream_metadata_counter')
 BEGIN SELECT RAISE(ABORT,'native stream observation cannot regress'); END;
CREATE TRIGGER native_stream_snapshot_reserved BEFORE DELETE ON native_stream_snapshots
 BEGIN SELECT RAISE(ABORT,'native stream observation identity remains reserved'); END;
PRAGMA user_version=112;
