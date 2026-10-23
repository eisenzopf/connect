#!/usr/bin/env python3
"""Recheck one owned database run and emit its actual Rust case result."""
import hashlib
import json
from pathlib import Path
import re
import sys
path = Path(sys.argv[1])
report = json.loads(path.read_text())
case = 'tests::live_put_get_delete_list_and_hash'
assert report['passed'] and report['sources_unchanged']
assert report['cleanup'] == {'container_removed': True, 'network_removed': True}
assert report['tests_passed'] == [case]
assert report['configuration']['bind'] == '127.0.0.1'
assert report['configuration']['no_image_pull']
text = (path.parent / 'test.log').read_text()
assert re.findall(r'^test (.+?) \.\.\. ok$', text, re.M) == [case]
with Path(report['executable']).open('rb') as stream:
    assert hashlib.file_digest(stream, 'sha256').hexdigest() == report['executable_sha256']
print(json.dumps({'passed': True, 'observed_case': case, 'cleanup_verified': True}))
