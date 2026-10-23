import assert from 'node:assert/strict';
import {test} from 'node:test';
import {ConnectError} from '../typescript/dist/index.js';
import {decodeResponse} from '../typescript/dist/protocol.js';

const reference={uuid:'record-one',session_id:'session-one',revision:1,state:'available',content_hash:'sha512-projection',valid_until:1793000000};
const item={sequence:4,kind:'record_ready',source_id:'record-one',actor_id:'owner',body:'{"uuid":"record-one"}',record:reference};
const source={source_id:'record-one',kind:'record_ready',actor_id:'owner',created_at:1792000000,provenance:'conversation-journal:event-one;current-authorized-record-projection',state:'available',body:item.body,record:reference};
const wrap=(kind,data)=>({profile:'connect-control/1',result:{Ok:{kind,data}}});
const conversation=row=>wrap('context_view',{conversation_id:'conversation-one',watermark:4,items:[row],omitted:0,deliveries:[],deliveries_omitted:0});
const task=row=>wrap('task',{kind:'recall',data:{sources:[row],next:null,omitted:0}});
const rejected=error=>error instanceof ConnectError&&error.code==='protocol';

test('record context preserves projected provenance and explicit unavailable states',()=>{
 for(const state of ['available','unavailable','deleted']) {
  const record={...reference,state,content_hash:state==='available'?reference.content_hash:null};
  const row={...item,record,body:state==='available'?item.body:''};
  assert.deepEqual(decodeResponse(conversation(row),'context_view').data.items[0],row);
  const taskRow={...source,record,state,body:state==='available'?source.body:null};
  assert.deepEqual(decodeResponse(task(taskRow),'task').data.data.sources[0],taskRow);
 }
});

test('record context rejects private fingerprints, mismatched source identity and false availability',()=>{
 for(const record of [
  {...reference,body_hash:'private-whole-record-hash'},
  {...reference,manifest_hash:'private-manifest-hash'},
  {...reference,uuid:'foreign-record'},
  {...reference,revision:0},
  {...reference,revision:Number.MAX_SAFE_INTEGER+1},
  {...reference,state:'deleted'},
  {...reference,content_hash:null},
  {...reference,valid_until:'tomorrow'},
  {...reference,valid_until:-1},
 ]) {
  assert.throws(()=>decodeResponse(conversation({...item,record}),'context_view'),rejected);
  assert.throws(()=>decodeResponse(task({...source,record}),'task'),rejected);
 }
});
