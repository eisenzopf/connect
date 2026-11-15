// Prospective complete RTC source-frame census. No bare receiver-RTP waiver.
export const RTC_SOURCE_FRAME_PROFILE='connect/native-rtc-original-source-frame/2';
export const RTC_ASSOCIATION_CONTROL_PROFILE='connect/rtc-passive-source-association/1';
export const RTC_WITNESS_CYCLE_BYTES=8*1024*1024;
const uint32=n=>Number.isSafeInteger(n)&&n>=0&&n<=0xffffffff;
const id=s=>typeof s==='string'&&s.length>0&&s.length<=256;
const freeze=value=>{if(value&&typeof value==='object'){for(const child of Object.values(value))freeze(child);Object.freeze(value);}return value;};
export function rtcSourceFrameControlRoute(plan){
 if(!Array.isArray(plan?.routes)||!Array.isArray(plan?.moves))throw Error('Original programme routes and moves required');
 const moved=new Set(plan.moves.map(move=>move.source));
 if([...moved].some(index=>!Number.isSafeInteger(index)||index<0))throw Error('Original moved route index required');
 const edges=plan.routes.map(route=>{const peers=plan.routes.filter(other=>other.session===route.session),at=peers.indexOf(route);
  const source=peers[(at+peers.length-1)%peers.length];return {receiver_route_index:route.index,source_route_index:source.index};})
  .filter(edge=>Number.isSafeInteger(edge.receiver_route_index)&&Number.isSafeInteger(edge.source_route_index)
   &&edge.receiver_route_index!==edge.source_route_index&&!moved.has(edge.receiver_route_index)&&!moved.has(edge.source_route_index))
  .sort((a,b)=>a.receiver_route_index-b.receiver_route_index);
 if(!edges.length)throw Error('Original unmoved source/receiver edge required');
 return Object.freeze({...edges[0],window_start_seconds:2,window_end_seconds:3});
}
// Fixed row layout: original row index, descriptor index, actual callback
// seconds, presentedFrames, original RTP timestamp, authored footer ordinal.
// Descriptor: original connection, stream, track, element ID. No image/PCM.
export function rtcSourceFrameCandidates(video,duplicates,omitted,nonDuplicatePassed,initialOrdinaryReceiver=false,associationControl=null,publicIntervals=[]){
 const descriptors=[],descriptorIndex=new Map(),rows=[],pairs=[],seenFooter=new Map(),seenRtp=new Set(),owners=new Map();
 let missing=0,repeatedRtp=0;
 const control=[];
 for(const row of video){const p=row.presentation,ordinal=row.sourceFrameIdentity?.ordinal;
  if(!p||p.profile!=='connect/rtc-video-presentation/1'||p.connection_id!==row.owner||!id(row.owner)
   ||!id(p.stream_id)||!id(p.track_id)||!uint32(p.rtpTimestamp)||!Number.isSafeInteger(p.element_id)||p.element_id<1
   ||!Number.isSafeInteger(p.presentedFrames)||p.presentedFrames<1||!Number.isSafeInteger(row.row_index)||row.row_index<0||row.row_index>=10000
   ||!Number.isFinite(row.seconds)||row.seconds<1||row.seconds>=61||!Number.isSafeInteger(ordinal)||ordinal<0||ordinal>=1860){missing++;continue;}
  const ownerKey=JSON.stringify([p.stream_id,p.track_id]),previous=owners.get(row.owner);
  if(previous!==undefined&&previous!==ownerKey){missing++;continue;}owners.set(row.owner,ownerKey);
  const descriptor=[row.owner,p.stream_id,p.track_id,p.element_id],key=JSON.stringify(descriptor);
  let d=descriptorIndex.get(key);if(d===undefined){d=descriptors.length;descriptorIndex.set(key,d);descriptors.push(descriptor);}
  const at=rows.length;rows.push([row.row_index,d,row.seconds,p.presentedFrames,p.rtpTimestamp,ordinal]);
  const rtpKey=JSON.stringify([row.owner,p.stream_id,p.track_id,p.rtpTimestamp]);if(seenRtp.has(rtpKey))repeatedRtp++;seenRtp.add(rtpKey);
  const first=seenFooter.get(ordinal);if(first!==undefined)pairs.push([first,at]);else seenFooter.set(ordinal,at);
  if(associationControl&&control.length<2&&row.seconds>=2&&row.seconds<=3)control.push(at);
 }
 const payload={descriptors,rows,pairs,control:associationControl?{...associationControl,rows:control}:null,
  public_intervals:structuredClone(publicIntervals)};
 const encoded=JSON.stringify(payload),encodedBytes=new TextEncoder().encode(encoded).byteLength;
 const validPair=([a,b])=>{const x=rows[a],y=rows[b];return x&&y&&x[1]===y[1]&&y[2]>x[2]&&y[3]>x[3]&&x[4]!==y[4];};
 const publicRow=row=>{const d=descriptors[row[1]];return publicIntervals.filter(interval=>interval.owner===d[0]
  &&interval.receive_stream_id===d[1]&&interval.start_ms/1000<=row[2]
  &&(interval.end_ms===null||row[2]<interval.end_ms/1000)).length===1;};
 const publicPair=([a,b])=>{const x=rows[a],y=rows[b],d=descriptors[x[1]];return publicIntervals.filter(interval=>interval.owner===d[0]
  &&interval.receive_stream_id===d[1]&&interval.start_ms/1000<=x[2]
  &&(interval.end_ms===null||y[2]<interval.end_ms/1000)).length===1;};
 const legacyCount=duplicates.length+omitted;
 const complete=video.length>0&&video.length<=10000&&rows.length===video.length&&descriptors.length<=10&&owners.size<=10
  &&missing===0&&repeatedRtp===0&&pairs.length===legacyCount&&rows.every(publicRow)&&pairs.every(pair=>validPair(pair)&&publicPair(pair))
  &&(!associationControl||(control.length===2&&validPair(control)))
  &&publicIntervals.length>0&&publicIntervals.length<=3&&encodedBytes<=RTC_WITNESS_CYCLE_BYTES;
 return freeze({profile:RTC_SOURCE_FRAME_PROFILE,state:'pending-original-capture-and-mapping',
  initial_ordinary_receiver:initialOrdinaryReceiver,complete,non_duplicate_passed:nonDuplicatePassed===true,
  legacy_draw_repeat_frames:legacyCount,legacy_diagnostic_omitted:omitted,repeated_receiver_rtp_frames:repeatedRtp,
  missing_presentation_rows:missing,rows:rows.length,pair_count:pairs.length,encoded_bytes:encodedBytes,
  census_json:encodedBytes<=RTC_WITNESS_CYCLE_BYTES?encoded:null,
  collection_passed:complete&&nonDuplicatePassed===true,qualified:false,
  scope:'Complete original public-row census; exact publication/epoch/closed-wire proof still mandatory. No FPS or source grant.'});
}
