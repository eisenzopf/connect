-- A private candidate's original context precedes any native allocation.
CREATE TABLE native_consultation_legs (
 connection_id TEXT PRIMARY KEY,
 operation_record_id TEXT NOT NULL UNIQUE REFERENCES operations(record_id),
 credential_id TEXT NOT NULL REFERENCES credentials(id),
 original_digest TEXT NOT NULL,
 runtime_epoch TEXT NOT NULL,
 endpoint_id TEXT NOT NULL REFERENCES endpoints(id),
 offer_json TEXT NOT NULL CHECK(json_valid(offer_json) AND length(offer_json)<=65536),
 audience_json TEXT NOT NULL CHECK(json_valid(audience_json) AND json_type(audience_json)='array'),
 primary_connection_id TEXT NOT NULL REFERENCES connections(id),
 endpoint_revision INTEGER NOT NULL,
 device_id TEXT NOT NULL REFERENCES devices(id)
);
CREATE TRIGGER native_consultation_leg_immutable BEFORE UPDATE ON native_consultation_legs
 BEGIN SELECT RAISE(ABORT,'private native context is immutable'); END;
CREATE TRIGGER native_consultation_leg_retained BEFORE DELETE ON native_consultation_legs
 BEGIN SELECT RAISE(ABORT,'private native context remains reserved'); END;
PRAGMA user_version=48;
