"""Explicit local package profiles and source-to-runtime asset verification.

Gate 4's inventory stays in package_gate4. Gate 5 adds deliverable assets and
requires its fixtures to match the source archive; inventory is never behavior
qualification. This module performs no builds, network access or account work.
"""
import hashlib
import json
import pathlib
import shutil
import tomllib

GATE5_RUNTIME = (
    'bin/connect-video', 'sdk/typescript/dist/browser_binding.js',
    'sdk/typescript/dist/browser_binding.d.ts', 'sdk/typescript/dist/browser_deadline.js',
    'sdk/typescript/dist/browser_deadline.d.ts', 'sdk/typescript/dist/browser_negotiation.js',
    'sdk/typescript/dist/browser_negotiation.d.ts', 'sdk/typescript/dist/browser_video.js',
    'sdk/typescript/dist/browser_video.d.ts', 'examples/conversation-client/video.js', 'examples/conversation-client/files.js',
    'sdk/typescript/dist/browser_video_transform.js',
    'sdk/typescript/dist/browser_video_keyframes.js', 'sdk/typescript/dist/browser_video_keyframes.d.ts', 'examples/conversation-client/contacts.js',
    'sdk/typescript/dist/media_diagnostics.js', 'sdk/typescript/dist/media_diagnostics.d.ts',
    'sdk/python/connect_sdk/media_validation.py',
    'contracts/fixtures/media-diagnostics-v1.json', 'docs/SETUP.md', 'docs/connect-first-ten-minutes.html', 'docs/CONNECTORS.md',
    'docs/PROTOCOL_CONFORMANCE.md', 'docs/GATE5_PACKAGE.md',
    'contracts/connect-control-v1.md', 'contracts/contact-pairing-v1.md',
    'contracts/connect-media-vp8-v1.md',
    'examples/native-video/README.md', 'examples/native-video/prepare-session.py',
    'examples/native-video/Cargo.toml', 'examples/native-video/src/main.rs',
    'examples/native-video/src/lib.rs', 'examples/native-video/src/audio.rs',
    'examples/native-video/src/interrupt.rs',
    'examples/native-video/src/preview.rs',
    'examples/native-video/src/report.rs',
    'examples/native-video/src/playout.rs',
    'examples/native-video/src/markers.rs',
    'contracts/connectors/profile-v1.md', 'contracts/connectors/host-context-v1.md',
    'contracts/fixtures/wire/framing-v1.json', 'contracts/fixtures/wire/rejections-v1.json',
    'examples/independent-uctp-client/client.py', 'examples/independent-uctp-client/test_client.py',
    'examples/independent-uctp-client/requirements.txt', 'examples/independent-uctp-client/README.md',
    'examples/independent-uctp-client/DEPENDENCIES.md',
    'examples/connector-template/Cargo.toml', 'examples/connector-template/Cargo.lock',
    'examples/connector-template/src/main.rs', 'examples/connector-template/src/lib.rs',
    'examples/connector-template/media/voice.rs',
    'examples/connector-contribution/Cargo.toml', 'examples/connector-contribution/Cargo.lock',
    'examples/connector-contribution/src/lib.rs', 'examples/connector-contribution/src/media.rs',
    'crates/connect-connector-api/Cargo.toml', 'crates/connect-connector-api/src/host_context.rs',
    'connector-kit-source.json',
)
GATE5_SOURCE = (
    'examples/native-video/Cargo.toml', 'examples/native-video/src/main.rs',
    'examples/native-video/src/lib.rs', 'sdk/typescript/src/browser_binding.ts',
    'sdk/typescript/src/browser_video.ts', 'sdk/typescript/src/browser_deadline.ts', 'sdk/typescript/src/browser_negotiation.ts',
    'crates/connect-server/tests/gate5_video_transport.rs',
    'crates/connect-server/tests/gate5_quic_transport.rs',
    'scripts/lib/prepared_archive.py', 'scripts/tests/test_prepared_archive.py',
    'scripts/check-parley.py', 'scripts/tests/test_gate5_dependency_roots.py',
    'crates/connect-server/tests/gate5_video_decode.rs',
    'crates/connect-server/tests/gate5_browser_video.rs',
    'crates/connect-server/tests/gate5_native_video.rs',
    'crates/connect-server/tests/gate5_native_browser_video.rs',
    'crates/connect-server/tests/gate5_native_diagnostics.rs',
    'crates/connect-server/tests/gate5_video_failures.rs',
    'crates/connect-server/tests/gate5_installed_video_failures.rs',
    'crates/connect-server/tests/gate5_support/installed_video.rs',
    'scripts/lib/media_failure_proof.py',
    'scripts/check-native-video-cleanup.py', 'scripts/tests/test_native_video_cleanup.py',
    'scripts/lib/native_process_owner.py',
    'crates/connect-client/src/video/diagnostics.rs',
    'crates/connect-server/tests/gate5_support/video.rs',
    'crates/connect-server/tests/gate5_support/video_codec.rs',
    'crates/connect-server/tests/secrets_native.rs',
    'crates/connect-server/tests/connector_template.rs',
    'crates/connect-server/tests/connector_contribution.rs',
    'scripts/check-independent-uctp.py', 'scripts/prepare-independent-uctp.py',
    'scripts/check-connector-contract.py', 'scripts/video/browser-qualify.mjs',
    'scripts/check-video-paths.py', 'scripts/video/native-processes.py',
    'scripts/video/decoder_observation.py', 'scripts/video/fixture_protocol.py',
    'scripts/video/source_identity.mjs', 'scripts/video/browser_failure.mjs',
    'scripts/video/marker_calibration.py', 'scripts/video/marker_calibration.mjs',
    'scripts/video/acceptance_policy.py', 'scripts/video/acceptance_policy.mjs',
    'scripts/tests/acceptance_policy.test.mjs', 'scripts/tests/av_acceptance_cases.json',
    'scripts/video/audio_observation.py', 'scripts/video/audio_observation.mjs',
    'scripts/video/audio_envelope.mjs', 'scripts/tests/audio_envelope.test.mjs',
    'scripts/video/motion_observation.py', 'scripts/video/motion_observation.mjs',
    'scripts/video/encoded_worker_observation.mjs', 'scripts/video/audio_spectrum.mjs',
    'sdk/typescript/src/browser_video_stats.ts', 'sdk/typescript/src/browser_video_transform.ts',
    'sdk/typescript/src/browser_video_setup.ts', 'sdk/typescript/src/browser_voice.ts',
    'sdk/typescript/src/browser_video_keyframes.ts', 'sdk/tests/video_keyframes.test.mjs',
    'sdk/tests/browser_voice.test.mjs',
    'sdk/tests/browser_rtx_offer.mjs',
    'sdk/tests/video_stats.test.mjs', 'sdk/tests/video_transform.test.mjs',
    'scripts/check-reference-client.mjs', 'scripts/lib/reference_inputs.mjs',
    'scripts/lib/reference_runtime.mjs', 'scripts/lib/reference_video.mjs',
    'scripts/lib/reference_continuity.mjs',
    'scripts/video/webrtc-impairment.py', 'scripts/tests/test_video_measurements.py',
    'scripts/lib/grant_boundary.py', 'scripts/tests/test_grant_boundary.py',
    'scripts/lib/package_environment.py',
    'scripts/check-gate5-setup.py', 'scripts/check-gate5-recovery.py',
    'scripts/lib/setup_provider_fixture.py',
    'scripts/check-connected-app.py', 'scripts/tests/test_connected_app_checker.py',
    'scripts/check-installed-sessions.py',
    'crates/connect-server/tests/gate5_contact_interface_parity.rs',
    'crates/connect-server/tests/gate5_session_interface_parity.rs',
    'sdk/tests/session_probe.py', 'sdk/tests/session_probe.mjs',
    'dependencies/recovery-baselines.json', 'scripts/build-recovery-baseline.py',
    'patches/recovery/schema1-identity-fixture.patch',
)
EXAMPLES = ('connector-template', 'connector-contribution', 'independent-uctp-client', 'native-video')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(root, names, label):
    for name in names:
        path = root / name
        if not path.is_file() or path.is_symlink():
            raise ValueError(f'Missing or nonordinary {label} asset: {name}')


def source_inventory(source):
    require(source, GATE5_SOURCE, 'Gate 5 source')
    require(source, [name for name in GATE5_RUNTIME if not name.startswith(('bin/', 'sdk/typescript/dist/'))
                     and name != 'connector-kit-source.json'], 'Gate 5 source')
    names = set(GATE5_SOURCE) | {name for name in GATE5_RUNTIME if not name.startswith(('bin/', 'sdk/typescript/dist/')) and name != 'connector-kit-source.json'}
    return {name: digest(source / name) for name in sorted(names)}


def standalone_api_manifest(source):
    """Flatten workspace inheritance, preserving the exact declared dependency values."""
    workspace = tomllib.loads((source / 'Cargo.toml').read_text())['workspace']
    original = tomllib.loads((source / 'crates/connect-connector-api/Cargo.toml').read_text())
    lines = ['[package]', 'name = "connect-connector-api"']
    for key in ('version', 'edition', 'rust-version', 'license', 'repository'):
        lines.append(f'{key} = {json.dumps(workspace["package"][key])}')
    lines += ['publish = false', '[workspace]']
    for section in ('dependencies', 'dev-dependencies'):
        lines.append(f'[{section}]')
        for name, declared in original.get(section, {}).items():
            value = workspace['dependencies'][name] if isinstance(declared, dict) and declared.get('workspace') else declared
            if isinstance(value, str):
                encoded = json.dumps(value)
            elif isinstance(value, dict):
                encoded = '{ ' + ', '.join(f'{key} = {json.dumps(item)}' for key, item in value.items()) + ' }'
            else:
                raise ValueError(f'Unsupported standalone API dependency: {name}')
            lines.append(f'{name} = {encoded}')
    lines += ['[lints.rust]', 'unsafe_code = "forbid"', '']
    return '\n'.join(lines)


def copy_gate5_assets(source, release):
    source_inventory(source)
    for name in EXAMPLES:
        shutil.copytree(source / 'examples' / name, release / 'examples' / name,
                        ignore=shutil.ignore_patterns('target', '__pycache__', '*.pyc'))
    api = release / 'crates/connect-connector-api'
    for name in ('src', 'tests'):
        shutil.copytree(source / 'crates/connect-connector-api' / name, api / name)
    (api / 'Cargo.toml').write_text(standalone_api_manifest(source))
    tracked = {}
    for directory in [release / 'examples' / name for name in EXAMPLES] + [api]:
        for path in sorted(directory.rglob('*')):
            if path.is_file() and path != api / 'Cargo.toml':
                relative = str(path.relative_to(release))
                tracked[relative] = digest(source / relative)
    (release / 'connector-kit-source.json').write_text(json.dumps({
        'profile': 'connect-connector-fixture-v1', 'source_files': tracked,
        'api_manifest': 'workspace inheritance flattened; dependency declarations preserved',
        'upstream_manifest_sha256': digest(source / 'crates/connect-connector-api/Cargo.toml'),
        'qualification': 'assets only; no behavioral or provider claim'}, indent=2) + '\n')


def inventory_gate5(release, source):
    require(release, GATE5_RUNTIME, 'Gate 5 runtime')
    source_inventory(source)
    kit = json.loads((release / 'connector-kit-source.json').read_text())
    if kit['upstream_manifest_sha256'] != digest(source / 'crates/connect-connector-api/Cargo.toml'):
        raise ValueError('Connector API manifest source mismatch')
    api_manifest = release / 'crates/connect-connector-api/Cargo.toml'
    if api_manifest.read_text() != standalone_api_manifest(source):
        raise ValueError('Flattened connector API dependency mismatch')
    expected = set()
    for directory in [source / 'examples' / name for name in EXAMPLES] + [source / 'crates/connect-connector-api/src', source / 'crates/connect-connector-api/tests']:
        for path in directory.rglob('*'):
            if path.is_file() and not any(part in ('target', '__pycache__') for part in path.relative_to(source).parts) and path.suffix != '.pyc':
                expected.add(str(path.relative_to(source)))
    if set(kit['source_files']) != expected:
        raise ValueError('Connector/independent source inventory mismatch')
    for name in expected:
        if digest(release / name) != digest(source / name) or kit['source_files'][name] != digest(source / name):
            raise ValueError(f'Packaged contributor source mismatch: {name}')
    for name in GATE5_RUNTIME:
        if name.startswith(('docs/', 'contracts/', 'examples/conversation-client/')) and digest(release / name) != digest(source / name):
            raise ValueError(f'Packaged Gate 5 asset differs from source: {name}')
    return {'profile': 'gate5-local', 'runtime_required': len(GATE5_RUNTIME),
            'source_required': len(GATE5_SOURCE), 'kit_files': len(expected),
            'behavior_qualified': False}


def verify_baseline(binary, manifest, source, schema):
    """Bind historical fixtures to the public catalog, including schema1's explicit patch."""
    document = json.loads((source / 'dependencies/recovery-baselines.json').read_text())
    catalog = document['baselines'][str(schema)]
    if not binary.is_file() or digest(binary) != manifest.get('binary_sha256'):
        raise ValueError(f'Schema {schema} baseline binary checksum mismatch')
    if manifest.get('schema') != schema or manifest.get('source_revision') != catalog['revision']:
        raise ValueError(f'Schema {schema} baseline public provenance mismatch')
    if manifest.get('source_repository') != document['repository'] or manifest.get('baseline_kind') != catalog['kind']:
        raise ValueError(f'Schema {schema} baseline repository/kind mismatch')
    if manifest.get('patch') != catalog.get('patch') or manifest.get('patch_sha256') != catalog.get('patch_sha256'):
        raise ValueError(f'Schema {schema} baseline patch mismatch')
    return {'schema': schema, 'source_revision': catalog['revision'],
            'binary_sha256': manifest['binary_sha256'], 'kind': catalog['kind']}


def copy_prepared_crate_notices(root, runtime, metadata):
    """Retain registry provenance after a recorded crate becomes a path dependency."""
    lock = json.loads((root / 'dependencies/source-lock.json').read_text())
    notices = []
    for name, source in lock['sources'].items():
        if source.get('kind') != 'crate':
            continue
        package = next((p for p in metadata['packages'] if p['name'] == name and
                        p['version'] == source['version']), None)
        expected = (root / '.deps' / name / 'Cargo.toml').resolve()
        if package is None or pathlib.Path(package['manifest_path']).resolve() != expected:
            raise ValueError(f'Prepared crate missing from selected package graph: {name}')
        identity = name + '-' + source['version']
        texts = []
        for path in expected.parent.iterdir():
            if path.is_file() and path.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'UNLICENSE')):
                target = runtime / 'licenses/prepared' / identity / path.name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, target)
                texts.append(str(target.relative_to(runtime)))
        for notice in source.get('license_files', []):
            path = root / notice['path']
            if digest(path) != notice['sha256']:
                raise ValueError('Prepared crate license digest mismatch')
            target = runtime / 'licenses/prepared' / identity / path.name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
            texts.append(str(target.relative_to(runtime)))
        notices.append({'package': identity, 'license': source['license'],
                        'source': source, 'texts': sorted(texts),
                        'patches': [p for p in lock['patches'] if p.get('target', 'rvoip') == name]})
    target = runtime / 'licenses/prepared-index.json'
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(notices, indent=2) + '\n')
    return notices
