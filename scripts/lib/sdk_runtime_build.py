"""Verify the native library selected by Cargo, including restored build caches."""
import hashlib
import json
from pathlib import Path
import re


def require_bundled_opus(proof):
    """Reject macOS artifacts that only recorded requested compiler settings."""
    if proof.get('target') != 'aarch64-apple-darwin':
        return
    native = proof.get('opus_native_build')
    if (not isinstance(native, dict) or native.get('mode') != 'bundled-static'
            or not isinstance(native.get('native_archive_sha256'), str)
            or not re.fullmatch('[0-9a-f]{64}', native['native_archive_sha256'])):
        raise ValueError('macOS runtime lacks verified bundled Opus provenance')


def bundled_opus(log):
    records = []
    for line in Path(log).read_text().splitlines():
        try:
            value = json.loads(line)
        except ValueError:
            continue
        if (value.get('reason') == 'build-script-executed'
                and value.get('package_id', '').rsplit('#', 1)[-1].split('@', 1)[0] == 'audiopus_sys'):
            records.append(value)
    if len(records) != 1:
        raise ValueError('A unique actual Opus native-build record is required')
    record, = records
    root = Path(record['out_dir']).resolve()
    paths = record.get('linked_paths', [])
    libraries = record.get('linked_libs', [])
    if 'static=opus' not in libraries or any(value in libraries for value in ('opus', 'dylib=opus')) or not paths:
        raise ValueError('macOS runtime requires the bundled static Opus build')
    archives = set()
    for value in paths:
        if not value.startswith('native='):
            raise ValueError('Unexpected Opus native library search kind')
        path = Path(value.removeprefix('native=')).resolve()
        archive = (path / 'libopus.a').resolve()
        if not path.is_relative_to(root) or not archive.is_relative_to(root) or not archive.is_file():
            raise ValueError('Cached Opus is not bundled; use a fresh or cleaned native build cache')
        archives.add(archive)
    if len(archives) != 1:
        raise ValueError('Ambiguous bundled Opus native archives')
    archive, = archives
    with archive.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    return {'mode': 'bundled-static', 'native_archive_sha256': digest}
