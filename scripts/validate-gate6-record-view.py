#!/usr/bin/env python3
"""Independently validate a generated local fixture view; no provider IO."""
import json
import importlib.metadata
from pathlib import Path
import sys
import jsonschema
root = Path(__file__).resolve().parents[1]
schema_path = root / '.deps/rvoip/crates/extensions/rvoip-vcon/tests/fixtures/vcon_json_schema.json'
schema = json.loads(schema_path.read_text())
target = Path(sys.argv[1])
paths = sorted(target.glob('*.json')) if target.is_dir() else [target]
if target.is_dir():
    assert {'audience-view.json', 'typed-reference-view.json'} <= {path.name for path in paths}
validator = jsonschema.Draft7Validator(schema, format_checker=jsonschema.FormatChecker())
for path in paths:
    value = json.loads(path.read_text())
    validator.validate(value)
print(json.dumps({'passed': True, 'jsonschema': importlib.metadata.version('jsonschema'), 'python': sys.version.split()[0], 'schema': str(schema_path.relative_to(root)), 'views': [str(path) for path in paths]}))
