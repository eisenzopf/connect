-- Semantic storage boundary: scope_json can now contain immutable reviewed SMS
-- purposes. Older binaries must not dispatch these tasks without checking them.
-- Existing task/grant bytes and empty-policy digests remain unchanged.
PRAGMA user_version=13;
