import copy
import dataclasses
import unittest
from connect_sdk.sdk_media import negotiation
from connect_sdk.sdk_media_types import MediaBinding
from connect_sdk.sdk_types import SdkError

class NativeVideoFeedbackCase(unittest.TestCase):
    def test_selected_allocation_is_immutable_and_strictly_scoped_in_media_description(self):
        binding = MediaBinding('conversation', 'session', 'connection')
        descriptor = dict(profile='rtcp-pli-epoch/1', original_operation='operation', original_offer='offer', stream_generation=1, connection_generation=2)
        wire = dict(type='negotiation', binding=binding._wire(), source_clock=True, stream_revision=1,
                    streams=[dict(id='camera', medium='video', direction='sendonly', codec={}, stream_local_id=7, video_feedback=descriptor)])
        read = lambda value: negotiation(binding, 0x81, value, b'')
        captured = read(wire).streams[0].video_feedback
        descriptor['original_offer'] = 'changed'
        self.assertEqual(captured.original_offer, 'offer')
        self.assertEqual(captured.original_operation, 'operation')
        with self.assertRaises(dataclasses.FrozenInstanceError): captured.stream_generation = 2
        for field, value in [('profile', 'other/1'), ('original_operation', None), ('original_offer', 'invalid offer'), ('stream_generation', True), ('stream_generation', 0), ('connection_generation', 9007199254740992), ('extra', True)]:
            bad = copy.deepcopy(wire); bad['streams'][0]['video_feedback'][field] = value
            with self.assertRaises((ValueError, SdkError)): read(bad)
        for field, value in [('medium', 'audio'), ('medium', 'screen'), ('medium', 'data'), ('stream_local_id', None), ('video_feedback', None)]:
            bad = copy.deepcopy(wire); bad['streams'][0][field] = value
            with self.assertRaises((ValueError, SdkError)): read(bad)
        for field in descriptor:
            bad = copy.deepcopy(wire); del bad['streams'][0]['video_feedback'][field]
            with self.assertRaises((ValueError, SdkError)): read(bad)

if __name__ == '__main__': unittest.main()
