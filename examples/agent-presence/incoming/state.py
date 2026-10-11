"""Private app-owned intake ledger; the Connect journal remains authoritative."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import uuid


def encode(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def digest(value):
    return hashlib.sha256(encode(value).encode()).hexdigest()


def private_directory(path):
    path = Path(path)
    path.mkdir(mode=0o700, parents=False, exist_ok=True)
    metadata = path.lstat()
    if not stat.S_ISDIR(metadata.st_mode) or metadata.st_mode & 0o077:
        raise ValueError("Intake state directory must be private and not a symlink")
    return path


class Ledger:
    """One process owns a bounded intake cursor, source records and exact intents."""

    def __init__(self, directory, binding):
        self.directory = private_directory(directory)
        fd = os.open(self.directory / "worker.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        self.lock = os.fdopen(fd, "r+")
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            database = self.directory / "intake.sqlite3"
            if database.is_symlink():
                raise ValueError("Intake database must not be a symlink")
            existing = database.exists()
            self.db = sqlite3.connect(database)
            if existing:
                if (self.db.execute("PRAGMA application_id").fetchone()[0] != 0x43494E42
                        or self.db.execute("PRAGMA user_version").fetchone()[0] != 1):
                    raise ValueError("Unknown intake database or unsupported schema; preserve it")
                if database.stat().st_mode & 0o077:
                    raise ValueError("Existing intake database must be private")
            else:
                os.chmod(database, 0o600)
                self.db.execute("PRAGMA application_id=1128877634")
                self.db.execute("PRAGMA user_version=1")
            self.db.execute("PRAGMA journal_mode=WAL")
            self.db.execute("PRAGMA synchronous=FULL")
            self.db.executescript("""
                CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY, digest TEXT NOT NULL, disposition TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS jobs(key TEXT PRIMARY KEY, source TEXT NOT NULL,
                    command TEXT NOT NULL, task TEXT, disposition TEXT NOT NULL DEFAULT 'pending');
            """)
            with self.db:
                saved = self.get("binding")
                if saved is None:
                    self.put("binding", digest(binding))
                elif saved != digest(binding):
                    raise ValueError("Intake identity or policy changed; preserve the original state")
        except BaseException:
            if hasattr(self, "db"):
                self.db.close()
            self.lock.close()
            raise

    def close(self):
        self.db.close()
        self.lock.close()

    def get(self, key):
        row = self.db.execute("SELECT value FROM metadata WHERE key=?", (key,)).fetchone()
        return json.loads(row[0]) if row else None

    def put(self, key, value):
        self.db.execute("INSERT OR REPLACE INTO metadata VALUES(?,?)", (key, encode(value)))

    def stage(self, page, classify, now):
        """Commit the inbox checkpoint only with durable work or an explicit disposition."""
        if len(page["events"]) > 100:
            raise ValueError("Inbox page exceeds intake limit")
        with self.db:
            for event in page["events"]:
                known = self.db.execute("SELECT digest FROM events WHERE id=?", (event["id"],)).fetchone()
                fingerprint = digest({key: value for key, value in event.items() if key != "cursor"})
                if known:
                    if known[0] != fingerprint:
                        raise ValueError("Retained event changed")
                    continue
                if self.db.execute("SELECT COUNT(*) FROM events").fetchone()[0] >= 10000:
                    raise ValueError("Intake ledger full; preserve it for maintenance")
                disposition, source, definition = classify(event, now)
                if source is not None:
                    key = digest([source["sender"], source["conversation_id"], source["request_id"]])
                    previous = self.db.execute("SELECT source FROM jobs WHERE key=?", (key,)).fetchone()
                    if previous:
                        original = json.loads(previous[0])
                        disposition = "duplicate" if original["body"] == source["body"] else "conflict"
                    else:
                        command = {"action": "create", "request_id": str(uuid.uuid4()), "definition": definition}
                        self.db.execute("INSERT INTO jobs(key,source,command) VALUES(?,?,?)",
                                        (key, encode(source), encode(command)))
                self.db.execute("INSERT INTO events VALUES(?,?,?)", (event["id"], fingerprint, disposition))
            self.put("cursor", page["cursor"])

    def pending(self):
        row = self.db.execute("SELECT key,source,command,task FROM jobs WHERE disposition='pending' ORDER BY rowid LIMIT 1").fetchone()
        if not row:
            return None
        return {"key": row[0], "source": json.loads(row[1]), "command": json.loads(row[2]),
                "task": json.loads(row[3]) if row[3] else None}

    def save_task(self, key, task):
        with self.db:
            previous = self.db.execute("SELECT task FROM jobs WHERE key=?", (key,)).fetchone()[0]
            if previous is not None and json.loads(previous)["id"] != task["id"]:
                raise ValueError("Task receipt changed identity")
            self.db.execute("UPDATE jobs SET task=? WHERE key=?", (encode(task), key))

    def finish(self, key, disposition):
        with self.db:
            self.db.execute("UPDATE jobs SET disposition=? WHERE key=?", (disposition, key))
