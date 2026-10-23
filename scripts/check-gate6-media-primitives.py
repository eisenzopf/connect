#!/usr/bin/env python3
"""Audit the G6-024 M1 primitive checklist from preserved component evidence.

This is a source/evidence audit, not a rerun or an aggregate test total. G6-028
owns the next current-source combined upstream qualification. Later installation,
operating/independent conformance and acceptance tasks retain their scope.
"""
import datetime
import hashlib
import json
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[1]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    components = {}
    cases = {}
    for key, expected in [('media-profiles', '0ac0e940c72f9813591f98a105c03bce201d682319a29a304beead3b75aa664c'), ('websocket-media', None)]:
        path = ROOT / f'docs/evidence/gate-6/{key}.json'
        report = json.loads(path.read_text())
        seal = json.loads((ROOT / f'.local/gate-6/{key}-verification.json').read_text())
        assert report['passed'] and report['sources_unchanged'] and not report['missing_required_cases']
        assert seal['passed'] and seal['report_sha256'] == digest(path)
        if expected:
            assert digest(path) == expected
        proof = ROOT / f'.local/gate-6/{key}-proof'
        assert (proof / 'checkpoint.json').read_bytes() == path.read_bytes()
        for name, want in report['source_inputs_sha256'].items():
            assert digest(proof / 'sources' / name) == want, name
        for step in report['steps']:
            assert step['passed'] and step['exit_code'] == 0
            assert digest(proof / (step['name'] + '.log')) == step['log_sha256']
        components[key] = dict(path=str(path.relative_to(ROOT)), sha256=digest(path),
                               scope=report['qualification'], source_inputs=len(report['source_inputs_sha256']),
                               runtime_tests=report['runtime_tests'], runtime_unique_tests=report['runtime_unique_tests'],
                               required_cases=len(report['required_cases']), steps=len(report['steps']))
        cases[key] = {name for step in report['steps'] for name in step['tests_passed']}
    current = json.loads((ROOT / 'docs/evidence/gate-6/websocket-media.json').read_text())
    for name, want in current['source_inputs_sha256'].items():
        assert digest(ROOT / name) == want, name
    baseline = json.loads((ROOT / '.local/gate-6/media-profiles-proof/sources/dependencies/source-lock.json').read_text())
    lock = json.loads((ROOT / 'dependencies/source-lock.json').read_text())
    assert len(lock['patches']) == 97 and lock['patches'][:96] == baseline['patches']
    for patch in lock['patches']:
        assert digest(ROOT / patch['path']) == patch['sha256']
    old = json.loads((ROOT / 'docs/evidence/gate-6/received-dtmf.json').read_text())
    assert digest(ROOT / 'docs/evidence/gate-6/received-dtmf.json') == '898b8184d5858198e867c671a515e074f9b8ef1b1b5b3becebadabe2bfa036ba'
    allowed = {'.deps/rvoip/crates/uctp/rvoip-websocket/src/media_bridge.rs',
               '.deps/rvoip/crates/webrtc/rvoip-webrtc/src/media/stream.rs',
               '.deps/rvoip/crates/webrtc/rvoip-webrtc/src/media/mod.rs'}
    unchanged = []
    changed = []
    for name, want in old['source_inputs_sha256'].items():
        if name.startswith('.deps/rvoip/') and '/src/' in name and name.endswith('.rs'):
            (unchanged if digest(ROOT / name) == want else changed).append(name)
    assert set(changed) == allowed, changed
    matrix = []

    def row(obligation, key, terms):
        selected = []
        for term in terms:
            found = sorted(name for name in cases[key] if term in name)
            assert found, (obligation, term)
            selected.extend(found)
        matrix.append(dict(obligation=obligation, component=key, named_cases=sorted(set(selected)), passed=True))

    row('Checked cross-runtime complete RTP framing', 'media-profiles', ['cross_runtime_media_vectors_match_checked_rust_and_browser_boundaries'])
    row('Physical-peer stream identity and exact route indexes', 'media-profiles', ['peer_ids_are_nonzero_monotonic_and_never_reused', 'route_session_and_connection_indexes_remove_exact_bindings'])
    row('Negotiated decoded PCMU, PCMA and mono Opus', 'media-profiles', ['actual_signed_native_three_audio_codecs_decode_across_unequal_mtu_peers_and_sequence_wrap'])
    row('Decoded camera/screen VP8, actual physical MTU fragmentation, sequence wrap, loss and recovery', 'media-profiles', ['actual_signed_native_camera_and_screen_refragment_to_physical_mtu_and_independently_decode_vp8'])
    row('Malformed, late and oversized refusal plus live recovery', 'media-profiles', ['actual_signed_native_malformed_oversized_and_expired_packets_are_isolated_and_live_audio_recovers'])
    row('Reliable ordered text/JSON/binary on QUIC, WebTransport and WSS', 'media-profiles', ['actual_signed_reliable_text_json_and_binary_profiles_validate_decode_and_recover_on_all_native_wires'])
    row('Explicit codec negotiation, atomic change and transcoding', 'media-profiles', ['configured_transcoder_honors_canonical_opus_mono', 'g711_transcoding_relabels_primary_payload_and_preserves_decodable_marker', 'actual_native_codec_update_wrong_confirmation_preserves_audio_and_retry_burns_provisional_handle'])
    row('Payload-owned feedback and primary repair', 'media-profiles', ['negotiated_audio_nack_does_not_replay_telephone_event_or_unknown_payloads_on_its_ssrc', 'declared_rtx_first_packet_reaches_primary_track_without_feedback_reset'])
    row('Live source/recipient DTMF attribution, original operation, method, timing and current permission', 'media-profiles', ['actual_received_sip_info_and_rtp_reach_signed_native_with_original_source_method_and_timing', 'actual_srtp_received_source_routes_through_core_with_exact_timing_and_fresh_signal_authority', 'native_core_recipient_stream_retirement_after_grant_prevents_actual_sink_poll'])
    row('Measured quality intervals, physical-peer accounting and no fabricated metrics', 'media-profiles', ['actual_native_default_quality_measures_accepted_rtp_without_fabricated_metrics', 'actual_native_tuned_quality_counts_only_current_accepted_stream_interval_and_idle_zeroes', 'actual_native_quality_measures_each_physical_recipient_datagram_and_not_its_peer'])
    row('Co-located fallback DTLS-SRTP mono audio with automatic delayed attachment and ten-frame queues', 'websocket-media', ['delayed_bidirectional_opus_is_automatically_attached_and_decodes_after_connection_wait', 'ws_to_ws_bridge_flows_frames_end_to_end'])
    row('Owned fallback teardown, SDP-envelope, trickle ICE and WSS boundaries', 'websocket-media', ['closing_idle_bridge_joins_track_attacher_and_cannot_recreate_media', 'envelope_only_sdp_exchange_completes_dtls_handshake', 'trickle_ice_candidate_api_round_trips_between_bridges', 'loopback_auth_handshake_over_wss'])
    result = dict(task='G6-024', passed=True, implementation_status='complete', full_task_qualification=True,
                  qualification='M1 reusable local macOS arm64 engine primitives against the G6-024 work/verification checklist. Component execution counts remain separate; this audit does not combine historical totals or assert one combined current-source runtime graph.',
                  executed_at=datetime.datetime.now(datetime.timezone.utc).isoformat(), components=components, checklist=matrix,
                  source_lock_sha256=digest(ROOT / 'dependencies/source-lock.json'), patches=97,
                  source_audit=dict(unchanged_prior_production_files=len(unchanged), changed_production_files=sorted(changed),
                                    change_scope='Optional fallback bridge ownership/bounds plus a public bounded-stream constructor delegating existing internals; patch96 adds fixture profiles. Prior Core/SIP/DTMF/transport algorithms are unchanged.'),
                  audit_script_sha256=digest(pathlib.Path(__file__)),
                  not_qualified=['G6-028 combined current-source upstream qualification and clean consumer preparation.',
                                 'Production canonical Store/journal/service/HTTP/SDK installation and public session orchestration in G6-037/G6-038 and dependent tasks.',
                                 'Live browser AV/fallback and extended operating/impairment/performance workloads, Linux x86_64 and named remote/provider G6-093 rehearsals, independent conformance, Gate 6 acceptance and publication.'])
    path = ROOT / 'docs/evidence/gate-6/media-primitives.json'
    path.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(dict(passed=True, task='G6-024', obligations=len(matrix), patches=97, source_audit=result['source_audit'])))


if __name__ == '__main__':
    main()
