"""Validate external-runtime AV failures without substituting embedded fixtures."""
import hashlib
import json
import math
import pathlib
import re

EXTERNAL_FAILURE_TESTS = frozenset({
    'installed_one_native_end_terminates_the_counterpart_and_allows_a_new_live_attachment',
    'installed_current_credential_revocation_ends_av_and_cannot_recreate_media_on_replay',
    'installed_durable_session_end_stops_both_streams_and_denies_a_stale_attachment',
    'installed_server_restart_reconciles_live_video_without_resurrecting_media_or_losing_context',
    'installed_physical_native_peer_loss_is_transport_failure_and_terminates_counterpart',
    'installed_edited_display_metadata_cannot_retarget_teardown_or_lose_owned_routes',
    'installed_another_client_cannot_cancel_or_retarget_the_owned_video_handle',
})


def digest(path):
    hasher = hashlib.sha256()
    with pathlib.Path(path).open('rb') as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b''):
            hasher.update(chunk)
    return hasher.hexdigest()


def verify_external_failures(result, directory, binary, source, revision):
    """Require every case and bind CLI/server and exact compiled fixture sources."""
    if result.get('rust_tests') != {'passed': 7, 'failed': 0, 'ignored': 0}:
        raise ValueError('All seven external-runtime media failure checks must execute')
    text = (directory / result['log']).read_text()
    rows = [json.loads(value) for value in re.findall(
        r'INSTALLED_VIDEO_FAILURE_RESULT (\{[^\n]*\})', text)]
    if len(rows) != 7 or {row.get('case') for row in rows} != EXTERNAL_FAILURE_TESTS:
        raise ValueError('Missing or duplicated external-runtime failure observation')
    selected = pathlib.Path(binary).resolve(strict=True)
    bound = {
        'selected_server_sha256': digest(selected),
        'source_revision': revision,
        'fixture_source_sha256': digest(source / 'crates/connect-server/tests/gate5_support/installed_video.rs'),
        'test_source_sha256': digest(source / 'crates/connect-server/tests/gate5_installed_video_failures.rs'),
    }
    for row in rows:
        if (row.get('status') != 'passed' or row.get('binary_rechecked') is not True
                or row.get('selected_server_binary') != str(selected)
                or any(row.get(key) != value for key, value in bound.items())
                or row.get('lifecycle_limit_seconds') != 5
                or type(row.get('public_cli_calls')) is not int or row['public_cli_calls'] < 3):
            raise ValueError('External-runtime failure proof did not bind selected inputs')
        for field in ('startup_seconds', 'shutdown_seconds'):
            times = row.get(field)
            if (not isinstance(times, list) or not times
                    or any(type(value) not in (int, float) or not math.isfinite(value)
                           or not 0 <= value < 5 for value in times)):
                raise ValueError('External-runtime failure lifecycle deadline not observed')
    return {'scope': 'public SDK compiled from matching qualification source; explicitly selected external server and CLI',
            **bound, 'cases': rows, 'passed': 7, 'failed': 0, 'ignored': 0}
