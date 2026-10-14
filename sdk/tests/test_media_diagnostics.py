"""Public health DTO validation shares only vectors, not implementation code."""
import copy
import json
from pathlib import Path
import unittest
from connect_sdk.errors import ConnectError
from connect_sdk.protocol import decode_response
from connect_sdk.media_validation import valid_media
FIXTURE=json.loads((Path(__file__).resolve().parents[2]/'contracts/fixtures/media-diagnostics-v1.json').read_text())


def decode(media):
    return decode_response(json.dumps({'profile':'connect-control/1','result':{'Ok':{'kind':'health','data':{
        'installation_id':'fixture','schema':17,'profile':'connect-control/1','readiness':{'state':'ready','components':[],'media':media}}}}}), 'health')


CONGESTION = {'congestion_samples': 2, 'maximum_sampled_connection_lost_packets': 11, 'maximum_sampled_connection_congestion_events': 9, 'maximum_sampled_connection_sent_packets': 496, 'minimum_sampled_rtt_us': 47000, 'maximum_sampled_rtt_us': 54000, 'minimum_sampled_cwnd_bytes': 6000, 'maximum_sampled_cwnd_bytes': 12000}

class MediaDiagnostics(unittest.TestCase):
    def test_valid_sampled_scope_and_nullable_absence(self):
        self.assertTrue(valid_media(FIXTURE))
        self.assertEqual(decode(FIXTURE)['data']['readiness']['media'], FIXTURE)
        current=copy.deepcopy(FIXTURE);current['quic'].update(CONGESTION)
        self.assertEqual(decode(current)['data']['readiness']['media'], current)
        value=copy.deepcopy(FIXTURE)
        del value['quic']
        del value['webrtc_video_input']
        value['queues'][0]['max_forwarded_frame_age_ms']=None
        value['queues'][0]['frame_age_boundary']=None
        self.assertTrue(valid_media(value))

    def test_bad_counter_sensitive_field_and_collection_bound_rejected(self):
        variants=[]
        for counter in (True,-1,0.5,9007199254740992,None):
            value=copy.deepcopy(FIXTURE);value['quic']['too_large']=counter;variants.append(value)
        value=copy.deepcopy(FIXTURE);value['quic']['remote_address']='private-diagnostic-secret';variants.append(value)
        value=copy.deepcopy(FIXTURE);value['queues']*=129;variants.append(value)
        value=copy.deepcopy(FIXTURE);value['queues'][0]['sampled_high_water_packets']=65;variants.append(value)
        value=copy.deepcopy(FIXTURE);value['queues'][0]['frame_age_boundary']=None;variants.append(value)
        value=copy.deepcopy(FIXTURE);value['forwarding'][0]['p50_us']=256;variants.append(value)
        value=copy.deepcopy(FIXTURE);value['webrtc_video_input']['source_packets']=True;variants.append(value)
        value=copy.deepcopy(FIXTURE);value['webrtc_video_input']['payload']='private-diagnostic-secret';variants.append(value)
        for key, invalid in [('congestion_samples', True), ('minimum_sampled_rtt_us', 90000), ('maximum_sampled_connection_lost_packets', -1), ('minimum_sampled_cwnd_bytes', None)]:
            value=copy.deepcopy(FIXTURE);value['quic'].update(CONGESTION);value['quic'][key]=invalid;variants.append(value)
        value=copy.deepcopy(FIXTURE);value['quic']['congestion_samples']=1;variants.append(value)
        for value in variants:
            with self.subTest(value=value):
                self.assertFalse(valid_media(value))
                with self.assertRaises(ConnectError) as error: decode(value)
                self.assertEqual(error.exception.code,'protocol')
                self.assertNotIn('private-diagnostic-secret',str(error.exception))

if __name__=='__main__': unittest.main()
