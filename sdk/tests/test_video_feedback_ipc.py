"""New typed helper adapter cases; synthetic IPC, not native decoder recovery."""
import asyncio
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'python'))
from connect_sdk import Client,ConversationId,SessionId,ConnectionId,StreamId,RequestOptions,RtpPacket,VideoReceiveEpoch,VideoFeedback,VideoKeyframePermit
from connect_sdk.sdk_media_types import MediaBinding,capture_rtp_video_epoch
from connect_sdk.sdk_video_feedback import capture_video_epoch,capture_video_feedback,take_video_feedback,consumed_video_feedback
ROOT=MediaBinding(ConversationId('room'),SessionId('session'),ConnectionId('connection'))
EPOCH={'epoch':'b'*32,'incarnation':'c'*32,'revision':2,'ssrc':83,'first_sequence':100,'first_timestamp':3000}

class VideoFeedbackIpc(unittest.IsolatedAsyncioTestCase):
    def test_history_and_references_keep_identity_and_original_round_trip_bound(self):
        self.assertIsInstance(capture_video_epoch(EPOCH),VideoReceiveEpoch)
        raw=bytes([0x80,96,0,100,0,0,11,184,0,0,0,83,0x10,0x31]);packet=RtpPacket(100,raw)
        capture_rtp_video_epoch(packet,EPOCH);self.assertEqual(packet.video_epoch.epoch,'b'*32);self.assertEqual(packet.data,raw)
        for field,value in [('epoch','0'*32),('incarnation','C'*32),('revision',0),('ssrc',0),('first_sequence',2**32),('extra',True)]:
            with self.assertRaises(ValueError):capture_video_epoch({**EPOCH,field:value})
        with self.assertRaises(ValueError):capture_rtp_video_epoch(RtpPacket(99,raw),EPOCH)
        owner=object();stream=StreamId('camera');metadata={'type':'video_feedback','binding':ROOT._wire(),'stream_id':stream,'token':'a'*32,'datagram_sequence':7,'ssrc':83,'epoch':'b'*32,'epoch_revision':2}
        hint=capture_video_feedback(owner,ROOT,stream,0x81,metadata,b'')
        self.assertIsInstance(hint,VideoFeedback)
        with self.assertRaises(ValueError):take_video_feedback(object(),ROOT,stream,hint)
        with self.assertRaises(ValueError):take_video_feedback(owner,ROOT,stream,VideoFeedback(7,83,'b'*32,2))
        token=take_video_feedback(owner,ROOT,stream,hint)
        with self.assertRaises(ValueError):take_video_feedback(owner,ROOT,stream,hint)
        reply={'type':'video_feedback_consumed','binding':ROOT._wire(),'stream_id':stream,'token':token,'remaining_ms':50}
        self.assertIsNone(consumed_video_feedback(ROOT,stream,hint,token,time.monotonic()-.06,0x81,reply,b''))
        permit=consumed_video_feedback(ROOT,stream,hint,token,time.monotonic(),0x81,reply,b'')
        self.assertIsInstance(permit,VideoKeyframePermit);self.assertTrue(permit.is_fresh());self.assertLessEqual(permit.expires_in_ms,50)
        with self.assertRaises(ValueError):consumed_video_feedback(ROOT,stream,hint,token,time.monotonic(),0x81,{**reply,'remaining_ms':201},b'')

    async def test_public_python_feedback_uses_owned_ipc_and_refuses_delayed_consumption(self):
        fixture=Path(__file__).parent/'fixtures/sdk_media_helper.py'
        for mode in ('feedback','feedback-delayed'):
            with tempfile.TemporaryDirectory() as tmp:
                root=Path(tmp);config=root/'client.json';config.write_text(json.dumps({'mode':mode,'pids':str(root/'pids'),'capture':str(root/'capture'),'release':str(root/'release')}))
                client=Client(config,fixture)
                try:
                    await client.capabilities.refresh(RequestOptions(5000))
                    stream=client.conversations.by_id(ROOT.conversation_id).sessions.by_id(ROOT.session_id).connections.by_id(ROOT.connection_id).media.stream_by_id(StreamId('camera'))
                    self.assertEqual((await stream.request_keyframe(RequestOptions(1000))).datagram_sequence,7)
                    self.assertEqual((await stream.receive_packet(RequestOptions(1000))).video_epoch.epoch,'b'*32)
                    hint=await stream.receive_video_feedback(RequestOptions(1000))
                    permit=await stream.consume_video_feedback(hint,RequestOptions(1000))
                    if mode=='feedback':self.assertTrue(permit.is_fresh())
                    else:self.assertIsNone(permit)
                    from connect_sdk import SdkError
                    with self.assertRaises(SdkError):await stream.consume_video_feedback(hint,RequestOptions(1000))
                    calls=[json.loads(v) for v in (root/'capture').read_text().splitlines()]
                    self.assertEqual(sum(v['channel']=='media' and v['request']['command']['action']=='consume_video_feedback' for v in calls),1)
                finally:
                    await client.close(RequestOptions(2000))
                    for pid in (root/'pids').read_text().splitlines():
                        with self.assertRaises(ProcessLookupError):os.kill(int(pid),0)

if __name__=='__main__':unittest.main()
