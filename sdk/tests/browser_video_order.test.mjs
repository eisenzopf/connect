import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserVideoOrder } from '../typescript/dist/browser_video_order.js';
import { MEDIA_LIMITS } from '../typescript/dist/browser_rtp.js';
const frame=(sequence,timestamp,arrived=performance.now(),ssrc=7)=>({bytes:new Uint8Array([1,2,3]),firstSequence:sequence,lastSequence:sequence,timestamp,ssrc,key:sequence===0,arrived});

test('complete video reordering preserves original clocks and avoids a false loss across RTP wrap',()=>{
 const delivered=[],owner=new BrowserVideoOrder((frames)=>delivered.push(...frames));
 try {
  const first=frame(65535,0xfffff448);owner.push(first,1000);
  const later=frame(1,3000);owner.push(later,3000);assert.equal(delivered.length,1);
  const middle=frame(0,0);owner.push(middle,2000);
  assert.deepEqual(delivered.map(x=>x.timestampUs),[1000,2000,3000]);
  assert.deepEqual(delivered.map(x=>x.frame),[first,middle,later]);
  assert.equal(owner.diagnostics.gaps,0);assert.equal(owner.queuedFrames,0);
  owner.push(middle,2000);assert.equal(delivered.length,3);assert.equal(owner.diagnostics.late,1);
 } finally { owner.close(); }
});

test('a real video gap waits only the bounded reorder interval and never renews original age',()=>{
 const delivered=[],expired=[],owner=new BrowserVideoOrder((frames,n)=>{delivered.push(...frames);expired.push(n);});
 try {
  const now=performance.now();owner.push(frame(10,30000,now),1000,now);
  owner.push(frame(12,36000,now),3000,now);owner.flush(now+19);assert.equal(delivered.length,1);
  owner.flush(now+20);assert.equal(delivered.length,2);assert.equal(owner.diagnostics.gaps,1);
  owner.push(frame(14,42000,now-190),5000,now);assert.equal(delivered.length,3,'assembly already consumed the reorder window');
  owner.push(frame(16,48000,now-200),7000,now);assert.equal(delivered.length,3);assert.equal(expired.at(-1),1);
 } finally { owner.close(); }
});

test('generation reset and shutdown discard pending video and cancel its timer',async()=>{
 const delivered=[],owner=new BrowserVideoOrder(frames=>delivered.push(...frames));
 owner.push(frame(10,30000),1000);owner.push(frame(12,36000),3000);assert.equal(owner.queuedFrames,1);
 owner.clear();owner.push(frame(2,6000),4000);assert.equal(delivered.length,2,'explicit new history accepts a lower sequence');
 owner.push(frame(4,12000),6000);owner.close();
 await new Promise(resolve=>setTimeout(resolve,30));
 assert.equal(delivered.length,2);assert.equal(owner.queuedFrames,0);
 owner.push(frame(5,15000),7000);assert.equal(delivered.length,2);
});

test('complete-frame pressure stays inside the frozen frame and fragment bounds',()=>{
 const delivered=[],owner=new BrowserVideoOrder(frames=>delivered.push(...frames));
 try {
  const now=performance.now();owner.push(frame(0,0,now),0,now);
  for(let i=2;i<30;i++) {
   owner.push(frame(i,i*3000,now),i*33333,now);
   assert.ok(owner.queuedFrames<=MEDIA_LIMITS.frames);
   assert.ok(owner.queuedFragments<=MEDIA_LIMITS.fragments);
  }
  owner.flush(now,true);assert.equal(owner.queuedFrames,0);
  assert.ok(delivered.every((x,i)=>!i||x.timestampUs>delivered[i-1].timestampUs));
 } finally { owner.close(); }
});
