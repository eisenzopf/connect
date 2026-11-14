// Prospective RTC source-frame census. Never a bare receiver-RTP exception.
export const RTC_SOURCE_FRAME_PROFILE='connect/native-rtc-original-source-frame/1';
const uint32=n=>Number.isSafeInteger(n)&&n>=0&&n<=0xffffffff;
const id=s=>typeof s==='string'&&s.length>0&&s.length<=256;
export function rtcSourceFrameCandidates(video,duplicates,omitted,nonDuplicatePassed,initialOrdinaryReceiver=false){
 const seen=new Set(),owners=new Map();let repeatedRtp=0,missing=0;
 for(const row of video){const p=row.presentation;
  if(!p||p.profile!=='connect/rtc-video-presentation/1'||p.connection_id!==row.owner
   ||!id(p.stream_id)||!id(p.track_id)||!uint32(p.rtpTimestamp)
   ||!Number.isSafeInteger(p.element_id)||p.element_id<1
   ||!Number.isSafeInteger(p.presentedFrames)||p.presentedFrames<1){missing++;continue;}
  const descriptor=JSON.stringify([p.stream_id,p.track_id]),old=owners.get(row.owner);
  if(old!==undefined&&old!==descriptor){missing++;continue;}owners.set(row.owner,descriptor);
  const key=JSON.stringify([row.owner,p.stream_id,p.track_id,p.rtpTimestamp]);
  if(seen.has(key))repeatedRtp++;seen.add(key);
 }
 const samePair=({first:a,current:b})=>a&&b&&a.owner===b.owner&&a.presentation&&b.presentation
  &&a.presentation.element_id===b.presentation.element_id
  &&a.presentation.stream_id===b.presentation.stream_id&&a.presentation.track_id===b.presentation.track_id
  &&b.presentation.presentedFrames>a.presentation.presentedFrames;
 const complete=video.length>0&&video.length<=10000&&owners.size<=10&&missing===0&&omitted===0
  &&duplicates.length<=16&&(!duplicates.length||initialOrdinaryReceiver)&&duplicates.every(samePair)&&repeatedRtp===0;
 const freeze=value=>{if(value&&typeof value==='object'){for(const child of Object.values(value))freeze(child);Object.freeze(value);}return value;};
 return freeze({profile:RTC_SOURCE_FRAME_PROFILE,state:'pending-original-capture-and-mapping',
  initial_ordinary_receiver:initialOrdinaryReceiver,complete,non_duplicate_passed:nonDuplicatePassed===true,legacy_draw_repeat_frames:duplicates.length+omitted,
  repeated_receiver_rtp_frames:repeatedRtp,missing_presentation_rows:missing,omitted,
  owners:[...owners].map(([connection_id,value])=>{const [stream_id,track_id]=JSON.parse(value);return {connection_id,stream_id,track_id};}),
  pairs:structuredClone(duplicates),
  collection_passed:complete&&nonDuplicatePassed===true,
  qualified:false,scope:'Immutable original public-window candidates only; unknown mapping remains fail. No FPS or source grant.'});
}
