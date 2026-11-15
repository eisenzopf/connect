"""Bounded supplemental controls; import the REAL helper after Root applies it.

These are header-metadata tests, never network, decoder or current-policy proof.
No copied HeaderIndex/prove_many implementation or replacement production path.
"""
import copy
import unittest
from unittest.mock import patch

from scripts.lib import rtc_source_frame_witness as witness
from scripts.tests.test_rtc_source_frame_witness import facts


class HeaderIndexSupplemental(unittest.TestCase):
    """Three isolated epoch/flow pairs, two original fragments per frame."""

    @staticmethod
    def fixture(*, wrap=False):
        _, _, _, _, source, target, original_epoch, _ = facts()
        rows, epochs = [], []
        for number in range(3):
            sf = (source[0], source[1] + number * 100, source[2],
                  source[3] + number * 100, source[4] + number * 10, source[5])
            tf = (target[0], target[1] + number * 100, target[2],
                  target[3] + number * 100, target[4] + number * 10, target[5])
            sequence_base = 65534 if wrap else 1000 + number * 100
            timestamp_base = (2**32 - 9000) if wrap else 3_000_000 + number * 1_000_000
            sequence_offset, timestamp_offset = 100 + number, 1000 + number * 111
            inputs = [(timestamp_base + i * 3000) % 2**32 for i in range(24)]
            outputs = [(value + timestamp_offset) % 2**32 for value in inputs]
            epoch = dict(original_epoch,
                source_connection='source_' + str(number),
                source_logical='source_video_' + str(number),
                source_transport='source_transport_' + str(number),
                source_generation=3 + number, source_ssrc=sf[4],
                target_logical='sink_video_' + str(number),
                target_transport='sink_transport_' + str(number),
                target_generation=2 + number, target_connection_generation=1 + number,
                target_ssrc=tf[4], input_sequence=sequence_base,
                input_timestamp=inputs[0], output_sequence=(sequence_base + sequence_offset) % 65536,
                output_timestamp=outputs[0], sequence_offset=sequence_offset,
                timestamp_offset=timestamp_offset,
                _end_output_timestamp=(outputs[-1] + 3000) % 2**32)
            requests = [(inputs[i:i + 2], outputs[i:i + 2]) for i in range(1, 21)]
            epochs.append((sf, tf, epoch, requests, inputs, outputs))
            for frame in range(24):
                for fragment in range(2):
                    seq = (sequence_base + frame * 2 + fragment) % 65536
                    source_time = (1_000_000_000 + number * 10_000_000_000
                                   + frame * 33_333_333 + fragment * 100_000)
                    for flow, sequence, timestamp, at in [
                        (sf, seq, inputs[frame], source_time),
                        (tf, (seq + sequence_offset) % 65536, outputs[frame], source_time + 5_000_000),
                    ]:
                        rows.append(dict(flow=flow, seq=sequence, ts=timestamp,
                            marker=fragment == 1, body_bytes=97 + fragment,
                            unix_ns=at, record=len(rows) + 1))
        return rows, epochs

    @staticmethod
    def index(rows, *, deadline=None):
        index = witness.HeaderIndex(maximum=288, deadline=deadline)
        for row in rows:
            index.add(row['flow'], row['seq'], row['ts'], row['marker'],
                      row['body_bytes'], row['unix_ns'], row['record'])
        return index.build()

    def test_twenty_aliases_each_in_three_isolated_epochs_keep_exact_two_fragment_bijections(self):
        rows, epochs = self.fixture()
        index = self.index(rows)
        self.assertEqual(len(index), 288)
        total = 0
        for sf, tf, epoch, requests, _, _ in epochs:
            self.assertGreater(len(requests), 16)
            results = index.prove_many(sf, tf, epoch, requests)
            self.assertEqual(len(results), 20)
            self.assertEqual([v['matched_packets'] for v in results], [8] * 20)
            self.assertEqual([v['original_timestamps'] for v in results], [v[0] for v in requests])
            self.assertEqual([v['target_timestamps'] for v in results], [v[1] for v in requests])
            total += len(results)
            index.release_work()
        self.assertEqual(total, 60)
        # Another real indexed target is never substituted for this flow/epoch.
        sf, _, epoch, requests, _, _ = epochs[0]
        with self.assertRaises(witness.WitnessError):
            index.prove_many(sf, epochs[1][1], epoch, requests)
        index.release_work()

    def test_missing_complete_target_frame_between_retained_endpoints_refuses_each_flow_only(self):
        rows, epochs = self.fixture()
        for missing in range(3):
            with self.subTest(epoch=missing):
                missing_tf, missing_ts = epochs[missing][1], epochs[missing][5][10]
                # Lose BOTH target fragments, keep the complete original source
                # frame, all neighbours and the two requested endpoint groups.
                retained = [r for r in rows if not (r['flow'] == missing_tf and r['ts'] == missing_ts)]
                self.assertEqual(len(retained), 286)
                index = self.index(retained)
                for number, (sf, tf, epoch, _, inputs, outputs) in enumerate(epochs):
                    requests = [([inputs[5], inputs[15]], [outputs[5], outputs[15]])]
                    if number == missing:
                        with self.assertRaisesRegex(witness.WitnessError, 'source-frame-missing-from-target'):
                            index.prove_many(sf, tf, epoch, requests)
                    else:
                        result = index.prove_many(sf, tf, epoch, requests)
                        self.assertEqual(result[0]['matched_packets'], 26)
                    index.release_work()

    def test_sequence_and_timestamp_wrap_with_reordered_headers_preserves_original_coordinates(self):
        rows, epochs = self.fixture(wrap=True)
        # Reverse arrival storage, including fragment order. Original RTP header
        # values and original numeric transit times stay exactly unchanged.
        before = copy.deepcopy(rows)
        index = self.index(list(reversed(rows)))
        for sf, tf, epoch, requests, inputs, outputs in epochs:
            self.assertGreater(inputs[2], inputs[3])
            self.assertTrue(any(r['flow'] == sf and r['seq'] == 0 for r in rows))
            result = index.prove_many(sf, tf, epoch, requests)
            self.assertEqual(len(result), 20)
            self.assertEqual([v['matched_packets'] for v in result], [8] * 20)
            self.assertEqual(result[0]['original_timestamps'], inputs[1:3])
            self.assertEqual(result[0]['target_timestamps'], outputs[1:3])
            index.release_work()
        self.assertEqual(rows, before)

    def test_selected_source_or_target_duplicate_is_never_guessed_away(self):
        rows, epochs = self.fixture()
        sf, tf, epoch, _, inputs, outputs = epochs[0]
        requests = [([inputs[5], inputs[15]], [outputs[5], outputs[15]])]
        for flow, timestamp in [(sf, inputs[10]), (tf, outputs[10])]:
            with self.subTest(flow=flow):
                duplicate = copy.deepcopy(next(r for r in rows if r['flow'] == flow and r['ts'] == timestamp))
                duplicate['record'] = 289
                # Count cap is not the reason for this refusal: the duplicated
                # original selected header must fail the actual frame proof.
                index = witness.HeaderIndex(maximum=289)
                for r in rows + [duplicate]:
                    index.add(r['flow'], r['seq'], r['ts'], r['marker'],
                              r['body_bytes'], r['unix_ns'], r['record'])
                index.build()
                with self.assertRaisesRegex(witness.WitnessError,
                        'original-frame-group-incomplete|source-duplicate-packet|target-duplicate-packet'):
                    index.prove_many(sf, tf, epoch, requests)
                index.release_work()

    def test_original_header_count_auxiliary_and_proof_scratch_caps_fail_closed(self):
        rows, epochs = self.fixture()
        row = rows[0]
        single = witness.HeaderIndex(maximum=1)
        args = (row['flow'], row['seq'], row['ts'], row['marker'], row['body_bytes'], row['unix_ns'], row['record'])
        single.add(*args)
        with self.assertRaisesRegex(witness.WitnessError, 'witness-global-header-cap'):
            single.add(*args)
        self.assertEqual(len(single), 1)
        # Charge retained metadata numerically; no giant allocation is made.
        auxiliary = witness.HeaderIndex(auxiliary_bytes=witness.HeaderIndex.LIMIT)
        with self.assertRaisesRegex(witness.WitnessError, 'closed-header-index-memory-bound'):
            auxiliary.check()
        index = self.index(rows)
        sf, tf, epoch, requests, _, _ = epochs[0]
        # Leave room for existing index + fixed slack, no room for transient
        # proof work. Exercise prove_many's actual reservation, not a fake list.
        with patch.object(witness.HeaderIndex, 'LIMIT', index.account + index.ALLOCATION_SLACK):
            with self.assertRaisesRegex(witness.WitnessError, 'closed-header-index-memory-bound'):
                index.prove_many(sf, tf, epoch, requests)
        self.assertEqual(len(index), 288)
        index.release_work()

    def test_original_deadline_refuses_build_and_alias_scan_without_waiting_or_renewal(self):
        rows, epochs = self.fixture()
        with patch.object(witness.time, 'monotonic', return_value=100.0) as clock:
            index = self.index(rows, deadline=110.0)
            sf, tf, epoch, requests, _, _ = epochs[0]
            self.assertEqual(len(index.prove_many(sf, tf, epoch, requests)), 20)
            index.release_work()
            clock.return_value = 110.0
            with self.assertRaisesRegex(witness.WitnessError, 'closed-metadata-time-bound'):
                index.prove_many(sf, tf, epoch, requests)
            index.release_work()
            pending = witness.HeaderIndex(deadline=110.0)
            with self.assertRaisesRegex(witness.WitnessError, 'closed-metadata-time-bound'):
                pending.build()
