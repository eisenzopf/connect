-- Fresh-state protocol boundary. Matching binaries coordinate authority writes
-- through owned per-resource file fences. Earlier candidate binaries must not
-- open this state and bypass that cooperative locking protocol.
PRAGMA user_version=29;
