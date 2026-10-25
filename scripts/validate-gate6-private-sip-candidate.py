#!/usr/bin/env python3
"""Validate actual reverse SIP-candidate captures and original-call correlations."""
import argparse
import hashlib
import json
from pathlib import Path
import jsonschema

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('captures', nargs=2, type=Path)
    parser.add_argument('--require-pending-return', action='store_true')
    args = parser.parse_args()
    rows = []
    digests = {}
    for path in args.captures:
        path = path.resolve()
        assert path.is_relative_to((ROOT / '.local').resolve())
        raw = path.read_bytes()
        assert len(raw) < 1024 * 1024
        rows.append(json.loads(raw))
        digests[path.name] = hashlib.sha256(raw).hexdigest()
    assert {row['algorithm'] for row in rows} == {'EdDSA', 'ES256'}
    schema_path = ROOT / 'contracts/schemas/uctp-envelope-2.schema.json'
    schema = json.loads(schema_path.read_text())
    envelope = jsonschema.Draft202012Validator(schema, format_checker=jsonschema.FormatChecker())
    operation = jsonschema.Draft202012Validator({'$defs': schema['$defs'], '$ref': '#/$defs/OperationUpdated'})
    seen = set()
    actual_frames = 0
    for row in rows:
        assert row['source_kind'] == 'private-initial-sip'
        private = row['selection']['private']
        assert len(private) == 2
        phone = next(p for p in private if p['connection_id'] == row['connection'])
        native = next(p for p in private if p['connection_id'] != row['connection'])
        remote = row['selection']['remote']['connection_id']
        assert phone['source_kind'] == row['source_kind'] and phone['generation'] == 1
        assert phone['record_id'] == row['child'] and phone['consultation_record_id'] == row['record']
        assert phone['runtime_connection_id'] == row['core'] and phone['sip_call_id'] == row['call']
        assert 'key_id' not in phone and 'credential_id' not in phone
        assert all(a != b for a, b in zip([row['connection'], row['core'], row['call']], row['public']))
        assert phone['connection_id'] != native['connection_id'] != remote
        assert all(p['connection_id'] != row['public'][0] for p in private)
        assert row['distinct_sip_calls'] == 2 and row['source_operation_count'] == 6
        assert all(n > 10 for n in row['baseline'])
        assert row['private_counts'][0] == 0 and row['private_counts'][1] > 10
        assert row['public_sibling_private_counts'] == [0, 0]
        assert row['private_native_to_sip_by_call'][0] > 10 and row['private_native_to_sip_by_call'][1] == 0
        assert row['admission_expired'] is True and row['controller_revoked'] is True
        assert row['after_expired_admission_and_revoked_controller'][0] == 0
        assert row['after_expired_admission_and_revoked_controller'][1] > 10
        assert all(n > 10 for n in row['recovered_public_counts'])
        completed = row['completed']
        operation.validate(completed)
        assert completed['state'] == 'completed' and completed['record_id'] == row['record']
        assert len(completed['effects']) == 1 and completed['effects'][0]['attempt'] == 1
        authority = row['authority']
        denied = authority['receive_denied']
        operation.validate(denied)
        assert denied['state'] == 'failed' and denied['problem']['code'] == 'signal-no-recipients'
        assert denied['problem']['signal']['recipients'] == []
        assert authority['receive_denied_ack'] and authority['source_denied_ack']
        assert authority['source_rows_before'] == 2 and authority['source_rows_after'] == 5
        assert authority['refused_source_rows_added'] == authority['refused_native_info_added'] == 0
        assert authority['native_send_refusal'] == 'forbidden'
        assert authority['phone_receive_denied']['state'] == authority['phone_receive_replay']['state'] == 'failed'
        assert authority['phone_receive_denied']['problem']['code'] == 'not-recipient'
        assert authority['phone_receive_denied']['problem'] == authority['phone_receive_replay']['problem']
        assert authority['phone_receive_denied_info_added'] == 0
        assert authority['media_counts_with_receive_denied'][0] == 0
        assert authority['media_counts_with_receive_denied'][1] > 10 and authority['media_marked_with_receive_denied'] > 10
        frames = [*row['private_frames'], authority['receive_recovered_frame'], authority['source_recovered_frame']]
        statuses = [*row['private_statuses'], authority['receive_recovered'], authority['source_recovered']]
        assert len(frames) == len(statuses) == 4
        assert len({saved['operation_id'] for saved in statuses}) == 4
        assert [f['payload']['source'] for f in row['private_frames']] == ['sip-info', 'rfc4733']
        for frame, saved in zip(frames, statuses):
            envelope.validate(frame)
            operation.validate(saved)
            assert frame['type'] == 'dtmf.received' and frame['signature']['alg'] == row['algorithm']
            assert frame['cid'] == row['conversation_id'] and frame['sid'] == 'native-session'
            assert frame['payload']['digits'] == '5' and frame['connid'] == native['connection_id']
            assert frame['operation_id'] == saved['operation_id'] and saved['state'] == 'completed'
            assert len(saved['result']['recipients']) == 1
            assert saved['result']['recipients'][0]['connection_id'] == native['connection_id']
            assert saved['result']['recipients'][0]['disposition'] == 'local_submitted'
            assert frame['id'] not in seen
            seen.add(frame['id'])
            actual_frames += 1
        sent = row['private_native_status']
        operation.validate(sent)
        assert sent['state'] == 'completed' and len(sent['result']['recipients']) == 1
        assert sent['result']['recipients'][0]['connection_id'] == row['connection']
        assert sent['result']['recipients'][0]['disposition'] == 'local_submitted'
        returned = row['returned']
        operation.validate(returned)
        assert returned['state'] == 'completed' and returned['result']['remote_continuing'] is True
        assert set(returned['result']['retained_connection_ids']) == {native['connection_id'], remote}
        assert returned['result']['retired_connection_ids'] == [row['connection']]
        assert len(returned['effects']) == 1 and returned['effects'][0]['attempt'] == 1
        assert ('pending_return' in row) == args.require_pending_return
        if args.require_pending_return:
            tail = row['pending_return']
            stopped = tail['stopped']
            operation.validate(stopped)
            assert stopped['state'] == 'unknown' and tail['replay_preserved'] is True
            summary = stopped['problem']['signal']
            assert summary['local_work_complete'] is True
            assert len(summary['recipients']) == 1
            assert summary['recipients'][0]['connection_id'] == row['connection']
            assert summary['recipients'][0]['disposition'] == 'outcome_unknown'
            assert tail['original_target_rows'] == 1
            assert 0 < tail['private_info_prefix_count'] < tail['scheduled_digits'] == 16
            assert tail['tail_quiet_ms'] == 2200 and tail['post_return_private_or_public_tail_count'] == 0
            assert len(tail['private_prefix_call_ids']) == tail['private_info_prefix_count']
            assert set(tail['private_prefix_call_ids']) == {row['call']}
            fresh = tail['fresh']
            operation.validate(fresh)
            assert fresh['state'] == 'completed'
            assert {r['connection_id'] for r in fresh['result']['recipients']} == {remote, row['public'][0]}
            assert all(r['disposition'] == 'local_submitted' for r in fresh['result']['recipients'])
            assert tail['fresh_info_call_id'] == row['public'][2]
            frame = tail['fresh_frame']
            envelope.validate(frame)
            assert frame['type'] == 'dtmf.received' and frame['signature']['alg'] == row['algorithm']
            assert frame['cid'] == row['conversation_id'] and frame['sid'] == 'native-session'
            assert frame['connid'] == remote and frame['operation_id'] == fresh['operation_id']
            assert frame['payload']['digits'] == '6' and frame['id'] not in seen
            assert frame['payload']['source'] == 'uctp-native' and frame['payload']['duration_ms'] == 100
            seen.add(frame['id'])
            actual_frames += 1
        restored = row['restored_status']
        operation.validate(restored)
        assert restored['state'] == 'completed'
        assert set(r['connection_id'] for r in restored['result']['recipients']) == {native['connection_id'], remote}
        assert len(restored['result']['recipients']) == 2 and len(row['restored_frames']) == 2
        assert all(restored['operation_id'] != saved['operation_id'] for saved in statuses)
        for frame in row['restored_frames']:
            envelope.validate(frame)
            assert frame['type'] == 'dtmf.received' and frame['signature']['alg'] == row['algorithm']
            assert frame['operation_id'] == restored['operation_id'] and frame['payload']['source'] == 'sip-info'
            assert frame['payload']['digits'] == '5' and frame['cid'] == row['conversation_id'] and frame['sid'] == 'native-session'
            assert frame['connid'] in {native['connection_id'], remote}
            assert frame['id'] not in seen
            seen.add(frame['id'])
            actual_frames += 1
        assert {f['connid'] for f in row['restored_frames']} == {native['connection_id'], remote}
    print(json.dumps({'passed': True, 'scenes': 2, 'actual_frames': actual_frames,
                      'pending_return_profiles': args.require_pending_return, 'captures_sha256': digests, 'schema_sha256': hashlib.sha256(schema_path.read_bytes()).hexdigest(),
                      'scope': 'Actual isolated local Human SIP candidate and native primary; SDK verifies signatures, runtime checks decoded duplex media, physical public sibling privacy, named signal rights/recovery/replay, expired admission/revoked controller continuity and original call return/absence. Other roles and provider profiles remain separate.'}))


if __name__ == '__main__':
    main()
