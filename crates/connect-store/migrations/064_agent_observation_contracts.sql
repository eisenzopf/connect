-- Frozen protected source restrictions survive transient buffer pruning.
-- This is provenance/anti-downgrade metadata, never a liveness or IO grant.
CREATE TABLE agent_observation_contracts (
 runtime_epoch TEXT NOT NULL,
 delivery_id TEXT NOT NULL REFERENCES context_deliveries(id),
 memory_kind TEXT NOT NULL CHECK(memory_kind IN ('public','private')),
 work_json TEXT,
 requires_activity INTEGER NOT NULL CHECK(requires_activity IN (0,1)),
 PRIMARY KEY(runtime_epoch,delivery_id)
);
PRAGMA user_version=64;
