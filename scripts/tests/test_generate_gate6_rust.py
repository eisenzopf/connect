"""Regenerate actual frozen UCTP payloads without weakening nullable contracts."""
from contextlib import redirect_stdout
from copy import deepcopy
import importlib.util
import io
import json
from pathlib import Path
import re
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / 'scripts/generate-gate6-rust.py'
SPEC = importlib.util.spec_from_file_location('gate6_rust_generator', SCRIPT)
GENERATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GENERATOR)
SCHEMA = ROOT / 'contracts/schemas/uctp-envelope-2.schema.json'
VECTORS = ROOT / 'contracts/uctp-vectors/gate6-envelope.json'


def struct_fields(source, name):
    match = re.search(r'pub struct ' + re.escape(name) + r' \{\n(.*?)\n\}', source, re.S)
    if match is None:
        raise AssertionError(f'missing generated struct {name}')
    return match.group(1)


class RequiredNullableGeneration(unittest.TestCase):
    def generate(self, schema=None):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            source_root = ROOT
            if schema is not None:
                source_root = temporary / 'source'
                schema_path = source_root / 'contracts/schemas/uctp-envelope-2.schema.json'
                vector_path = source_root / 'contracts/uctp-vectors/gate6-envelope.json'
                schema_path.parent.mkdir(parents=True)
                vector_path.parent.mkdir(parents=True)
                schema_path.write_text(json.dumps(schema))
                vector_path.write_bytes(VECTORS.read_bytes())
            output = temporary / 'generated'
            with patch.object(GENERATOR, 'ROOT', source_root), redirect_stdout(io.StringIO()):
                GENERATOR.generate(output)
            return {name: (output / name).read_bytes() for name in
                    ('payloads.rs', 'envelope.schema.json', 'golden-vectors.json')}

    def test_frozen_required_nullable_silence_keeps_presence_and_identifier_type(self):
        schema = json.loads(SCHEMA.read_text())
        required_nullable = [
            (name, key)
            for name, definition in schema['$defs'].items()
            for key, field in definition.get('properties', {}).items()
            if key in definition.get('required', [])
            and any(branch.get('type') == 'null' for branch in field.get('anyOf', []))
        ]
        self.assertEqual(required_nullable, [('StreamActiveSpeaker', 'active_participant')])
        source = self.generate()['payloads.rs'].decode()
        fields = struct_fields(source, 'StreamActiveSpeaker')
        self.assertIn(
            '#[serde(deserialize_with = "deserialize_required_nullable")]\n'
            '    pub active_participant: Option<Identifier>,', fields)
        self.assertNotIn('default', fields)
        self.assertNotIn('skip_serializing_if', fields)
        self.assertIn('pub strm_id: Identifier,', fields)
        self.assertIn('Option::<T>::deserialize(deserializer)', source)
        self.assertIn("T: Deserialize<'de>", source)

    def test_optional_nullable_fields_keep_one_option_and_existing_omission(self):
        source = self.generate()['payloads.rs'].decode()
        for name, field, inner in [
            ('AuthResponse', 'actor_token', 'String'),
            ('MessageSend', 'in_reply_to_msg', 'Identifier'),
            ('RecordingVconFetched', 'expires_at', 'DateTime<Utc>'),
            ('ConversationCreate', 'idle_close_secs', 'u64'),
            ('SessionEnded', 'vcon_handle', 'Value'),
        ]:
            with self.subTest(struct=name, field=field):
                fields = struct_fields(source, name)
                self.assertIn(
                    '#[serde(default, skip_serializing_if = "Option::is_none")]\n'
                    f'    pub {field}: Option<{inner}>,', fields)
                self.assertNotIn('deserialize_required_nullable', fields)
        self.assertNotIn('Option<Option<', source)
        self.assertIn('pub operation_id: Identifier,', struct_fields(source, 'OperationGet'))

    def test_nullable_union_branch_order_cannot_change_the_generated_type(self):
        schema = json.loads(SCHEMA.read_text())
        changed = deepcopy(schema)
        field = changed['$defs']['StreamActiveSpeaker']['properties']['active_participant']
        field['anyOf'].reverse()
        normal = self.generate()['payloads.rs'].decode()
        reversed_null = self.generate(changed)['payloads.rs'].decode()
        self.assertEqual(struct_fields(normal, 'StreamActiveSpeaker'),
                         struct_fields(reversed_null, 'StreamActiveSpeaker'))

    def test_regeneration_keeps_actual_schema_and_null_numeric_missing_vectors(self):
        output = self.generate()
        self.assertEqual(output['envelope.schema.json'], SCHEMA.read_bytes())
        self.assertEqual(output['golden-vectors.json'], VECTORS.read_bytes())
        vectors = {row['id']: row for row in json.loads(output['golden-vectors.json'])['vectors']}
        silence = vectors['active-speaker-session-silence']
        absent = vectors['active-speaker-needs-explicit-participant-or-null']
        numeric = vectors['active-speaker-refuses-numeric-participant']
        self.assertIs(silence['valid'], True)
        self.assertIsNone(silence['envelope']['payload']['active_participant'])
        self.assertIs(absent['valid'], False)
        self.assertNotIn('active_participant', absent['envelope']['payload'])
        self.assertIs(numeric['valid'], False)
        self.assertEqual(numeric['envelope']['payload']['active_participant'], 37)


if __name__ == '__main__':
    unittest.main()
