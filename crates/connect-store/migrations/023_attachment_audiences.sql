-- Fresh candidate boundary: stored artifact_begin intents now require an
-- explicit participant audience. Older task/receipt JSON must never be silently
-- interpreted under the new upload policy; prior databases are refused unchanged.
PRAGMA user_version=23;
