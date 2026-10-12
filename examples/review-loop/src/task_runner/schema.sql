CREATE TABLE runner_metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE runner_events(id TEXT PRIMARY KEY, sequence INTEGER NOT NULL UNIQUE);
CREATE TABLE runner_actions(reply_id TEXT PRIMARY KEY, kind TEXT NOT NULL);
CREATE TABLE runner_commands(
  sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  key TEXT NOT NULL UNIQUE,
  lane TEXT NOT NULL CHECK(lane IN ('controller','invocation')),
  command TEXT NOT NULL,
  outcome TEXT
);
CREATE TABLE runner_rejections(
  sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  command_key TEXT NOT NULL REFERENCES runner_commands(key),
  code TEXT NOT NULL
);
PRAGMA user_version=1;
