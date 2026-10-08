CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    closed INTEGER NOT NULL DEFAULT 0 CHECK (closed IN (0,1))
);
CREATE TABLE participants (
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    identity_id TEXT NOT NULL REFERENCES identities(id),
    PRIMARY KEY (conversation_id, identity_id)
);
CREATE TABLE messages (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    sender TEXT NOT NULL REFERENCES identities(id),
    body TEXT NOT NULL
);
CREATE INDEX messages_conversation_sequence ON messages(conversation_id, sequence);
CREATE TABLE requests (
    actor TEXT NOT NULL REFERENCES identities(id),
    id TEXT NOT NULL,
    digest TEXT NOT NULL,
    operation TEXT NOT NULL,
    resources TEXT NOT NULL,
    result TEXT NOT NULL,
    PRIMARY KEY (actor, id)
);
CREATE TABLE journal (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    recipient TEXT NOT NULL REFERENCES identities(id),
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    kind TEXT NOT NULL,
    message_id TEXT REFERENCES messages(id)
);
CREATE INDEX journal_recipient_sequence ON journal(recipient, sequence);
PRAGMA user_version = 2;
