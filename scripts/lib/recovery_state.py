"""Read-only migration comparisons and synthetic, account-free recovery fixtures."""
import hashlib
import json
import sqlite3
import time


def quote(name):
    return '"' + name.replace('"', '""') + '"'


def snapshot(path):
    """Keep original columns/rows; additive columns cannot conceal lost data."""
    with sqlite3.connect(f'file:{path}?mode=ro', uri=True) as db:
        tables = [r[0] for r in db.execute(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")]
        return {table: {
            'columns': [r[1] for r in db.execute(f'PRAGMA table_info({quote(table)})')],
            'rows': sorted(db.execute(f'SELECT * FROM {quote(table)}').fetchall(), key=repr),
        } for table in tables}


def preserved(path, original, schema):
    with sqlite3.connect(f'file:{path}?mode=ro', uri=True) as db:
        assert db.execute('PRAGMA user_version').fetchone()[0] == schema
        assert db.execute('PRAGMA integrity_check').fetchone()[0] == 'ok'
        assert not db.execute('PRAGMA foreign_key_check').fetchall()
        for table, saved in original.items():
            columns = ','.join(quote(c) for c in saved['columns'])
            rows = sorted(db.execute(f'SELECT {columns} FROM {quote(table)}').fetchall(), key=repr)
            assert rows == saved['rows'], f'Original rows changed: {table}'


def sms_fixture(path, cid):
    """Seed synthetic policy rows only; this does not prove SMS dispatch or consent."""
    now = int(time.time())
    with sqlite3.connect(path) as db:
        db.execute('INSERT INTO sms_authorities VALUES(?,?,?,?)',
                   ('offline-recovery', 1, 'fixture-digest', now + 1800))
        db.execute('INSERT INTO sms_grants VALUES(?,?,?,?,?,?,?,?)',
                   ('offline-recovery', 'fixture-grant', '+15550000001', '+15550000002',
                    cid, 'offline-only-evidence', now + 1800, 1))
        db.execute('INSERT INTO sms_controls VALUES(?,?,?,?)',
                   ('+15550000001', 1, now, 'fixture-stop'))
        db.execute('INSERT INTO sms_control_events VALUES(?,?,?,?,?,?)',
                   ('fixture-stop', '+15550000001', '+15550000002', 'STOP', now, 1))


def baseline(binary, revision, schema, manifest_path=None):
    manifest = json.loads((manifest_path or binary.with_name('manifest.json')).read_text())
    assert manifest['source_revision'] == revision, 'Wrong historical source'
    assert manifest['schema'] == schema, 'Wrong historical schema'
    expected = manifest.get('binary_sha256') or manifest['files']['bin/connect']
    actual = hashlib.sha256(binary.read_bytes()).hexdigest()
    assert actual == expected, 'Historical executable digest mismatch'
    return {'source_revision': revision, 'schema': schema, 'binary_sha256': actual}
