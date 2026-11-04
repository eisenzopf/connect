"""Focused helper-decoding proof for the current native activation result."""
import json
from pathlib import Path
from copy import deepcopy
import unittest
from connect_sdk.sdk_movements import move_result
from connect_sdk import OperationKey, OperationId, Scope

class Handles:
    @property
    def conversations(self): return self
    @property
    def sessions(self): return self
    @property
    def connections(self): return self
    def by_id(self, id): return Handle(id)
class Handle(Handles):
    def __init__(self,id): self.id=id

class NativeMoveActivation(unittest.TestCase):
    def test_native_activation_preserves_stopped_history_and_rejects_other_overlap(self):
        ready=json.loads((Path(__file__).parent/'fixtures/native_move_activation.json').read_text())
        handle=lambda id:dict(id=id,conversationId='conversation',sessionId='session',receiptStreamRevision=None)
        base=dict(retained=[handle('remote')],admitted=[handle('candidate')],retired=['source'],topologyRevision=4,conversationRevision=None,
                  connection=handle('candidate'),activationState='ready',binding=dict(generation=2,participantId='publisher',membershipEpoch=9,endpointId='endpoint_new',deviceId='device_new'))
        self.assertEqual(base['activationState'],ready['activation_state'])
        key=OperationKey(OperationId('original'),'connection.replace',Scope('conversation','session','source'))
        saved={'continuation':dict(kind='replacement',retained_connection_id='remote',target=dict(kind='endpoint',id='endpoint_new'))}
        decode=lambda v:move_result(v,Handles(),key,saved)
        for state in ['ready','stopped',None]:
            v=deepcopy(base);v['activationState']=state
            if state=='stopped':v['retired'].append('candidate')
            result=decode(v);self.assertEqual(result.activation_state,state);self.assertEqual(result.retired,tuple(v['retired']))
        stopped=deepcopy(base);stopped['activationState']='stopped';stopped['retired'].append('candidate')
        for field,value in [('activationState','prepared'),('activationState',None),('activationState',[]),
                            ('retired',['source']),('retired',['source','candidate','candidate']),('retired',['source','candidate','remote']),('admitted',[handle('foreign')])]:
            v=deepcopy(stopped);v[field]=value
            with self.assertRaises(ValueError):decode(v)
        missing=deepcopy(base);del missing['activationState']
        with self.assertRaises(ValueError):decode(missing)
