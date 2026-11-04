"""Original helper-owned feedback references; no media permission or renewed age."""
from __future__ import annotations
from dataclasses import dataclass
import math
import re
import time
from weakref import WeakKeyDictionary

def shape(value, keys):
    if type(value) is not dict or set(value) != set(keys):
        raise ValueError()
    return value

def unsigned(value, positive=False):
    if type(value) is not int or not int(positive) <= value <= 0xffffffff:
        raise ValueError()
    return value

def revision(value):
    if type(value) is not int or not 1 <= value <= 9_007_199_254_740_991:
        raise ValueError()
    return value

def hex_id(value):
    if type(value) is not str or re.fullmatch('[0-9a-f]{32}',value) is None or value == '0'*32:
        raise ValueError()
    return value

@dataclass(frozen=True)
class VideoReceiveEpoch:
    epoch: str
    incarnation: str
    revision: int
    ssrc: int
    first_sequence: int
    first_timestamp: int

def capture_video_epoch(value):
    shape(value,('epoch','incarnation','revision','ssrc','first_sequence','first_timestamp'))
    return VideoReceiveEpoch(hex_id(value['epoch']),hex_id(value['incarnation']),revision(value['revision']),
                             unsigned(value['ssrc'],True),unsigned(value['first_sequence']),unsigned(value['first_timestamp']))

@dataclass(frozen=True,eq=False,repr=False)
class VideoFeedback:
    datagram_sequence: int
    ssrc: int
    epoch: str
    epoch_revision: int

@dataclass(frozen=True,repr=False)
class VideoKeyframePermit:
    feedback: VideoFeedback
    _deadline: float

    def is_fresh(self) -> bool:
        return time.monotonic() < self._deadline

    @property
    def expires_in_ms(self) -> float:
        return max(0,(self._deadline-time.monotonic())*1000)

_references=WeakKeyDictionary()

def root(binding,stream,kind,value,payload):
    if kind != 0x81 or payload or type(value) is not dict or value.get('binding') != binding._wire() or value.get('stream_id') != stream:
        raise ValueError()

def key(binding,stream):
    return (binding.conversation_id,binding.session_id,binding.connection_id,stream)

def capture_video_feedback(owner,binding,stream,kind,value,payload):
    root(binding,stream,kind,value,payload)
    shape(value,('type','binding','stream_id','token','datagram_sequence','ssrc','epoch','epoch_revision'))
    if value['type'] != 'video_feedback':
        raise ValueError()
    token=hex_id(value['token'])
    result=VideoFeedback(unsigned(value['datagram_sequence']),unsigned(value['ssrc'],True),hex_id(value['epoch']),revision(value['epoch_revision']))
    _references[result]=(owner,key(binding,stream),token)
    return result

def take_video_feedback(owner,binding,stream,value):
    if type(value) is not VideoFeedback:
        raise ValueError()
    ref=_references.get(value)
    if ref is None or ref[0] is not owner or ref[1] != key(binding,stream):
        raise ValueError()
    del _references[value]
    return ref[2]

def consumed_video_feedback(binding,stream,feedback,token,started,kind,value,payload):
    root(binding,stream,kind,value,payload)
    shape(value,('type','binding','stream_id','token','remaining_ms'))
    if (value['type'] != 'video_feedback_consumed' or value['token'] != token
            or type(value['remaining_ms']) is not int or not 0 <= value['remaining_ms'] <= 200
            or not math.isfinite(started) or started > time.monotonic()):
        raise ValueError()
    # Charge the complete monotonic RPC elapsed time, including local queuing.
    # Response arrival must never become a new 200ms origin.
    deadline=started+value['remaining_ms']/1000
    return VideoKeyframePermit(feedback,deadline) if time.monotonic()<deadline else None
