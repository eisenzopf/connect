/** Bounded public peer state diagnostics. Never retain SDP, addresses or credentials. */
export function installPeerDiagnostics({peerBound=4}={}) {
    if(!Number.isInteger(peerBound)||peerBound<1||peerBound>16)throw Error('Bounded original peer diagnostics');
    const NativePeer = globalThis.RTCPeerConnection;
    const entries = [];
    let omittedPeers = 0;
    const state = (value, allowed) => allowed.includes(value) ? value : "other";
    const sample = (peer) => ({
        connection: state(peer.connectionState, ["new", "connecting", "connected", "disconnected", "failed", "closed"]),
        ice: state(peer.iceConnectionState, ["new", "checking", "connected", "completed", "disconnected", "failed", "closed"]),
        gathering: state(peer.iceGatheringState, ["new", "gathering", "complete"]),
        signaling: state(peer.signalingState, ["stable", "have-local-offer", "have-remote-offer", "have-local-pranswer", "have-remote-pranswer", "closed"]),
        local_description_set: Boolean(peer.localDescription),
        remote_description_set: Boolean(peer.remoteDescription),
        dtls: [...new Set(peer.getTransceivers().flatMap(transceiver =>
            [transceiver.sender?.transport?.state, transceiver.receiver?.transport?.state])
            .filter(value => value !== undefined))].slice(0, 4)
            .map(value => state(value, ["new", "connecting", "connected", "closed", "failed"])),
    });
    globalThis.RTCPeerConnection = class extends NativePeer {
        constructor(...args) {
            super(...args);
            if (entries.length >= peerBound) { omittedPeers++; return; }
            const entry = {peer: this, peer_index:entries.length, connection_id:null, current:null, codecs:null, detach:null, stats_ids:new Map(), omitted_media_rows:0, omitted_listeners:0, events: [], omitted_events: 0, snapshot_errors: 0};
            entries.push(entry);
            const observedTransports = new WeakSet(),listeners=[];
            const listen=(target,event,callback)=>{
                if(listeners.length>=20){entry.omitted_listeners++;return;}
                target.addEventListener(event,callback);listeners.push([target,event,callback]);
            };
            entry.detach=()=>{for(const [target,event,callback]of listeners)target.removeEventListener?.(event,callback);listeners.length=0;};
            const record = () => {
                if(entry.peer!==this)return;
                try {
                    const value = {...sample(this), elapsed_ms: performance.now()};
                    if (entry.events.length < 128) entry.events.push(value);
                    else entry.omitted_events++;
                    for (const transceiver of this.getTransceivers()) {
                        for (const transport of [transceiver.sender?.transport, transceiver.receiver?.transport]) {
                            if (transport && !observedTransports.has(transport)) {
                                observedTransports.add(transport);
                                listen(transport,"statechange", record);
                            }
                        }
                    }
                } catch { entry.snapshot_errors++; }
            };
            for (const event of ["connectionstatechange", "iceconnectionstatechange", "icegatheringstatechange", "signalingstatechange"])
                listen(this,event, record);
            record();
        }
        close(){
            const entry=entries.find(value=>value.peer===this);
            if(entry)try{entry.codecs=codecSnapshot(this);}catch{entry.codecs={unavailable:true};}
            try{return super.close();}
            finally{
                if(entry&&(this.connectionState==='closed'||this.signalingState==='closed')){
                    try{entry.current=sample(this);}catch{entry.current={unavailable:true};}
                    entry.detach?.();entry.detach=null;entry.peer=null;entry.stats_ids.clear();
                }
            }
        }
    };
    globalThis.connectPeerTag=(peer,connectionId)=>{
        const entry=entries.find(value=>value.peer===peer);
        if(!entry||typeof connectionId!=='string'||!connectionId.length||connectionId.length>256||
            entry.connection_id!==null&&entry.connection_id!==connectionId)return false;
        entry.connection_id=connectionId;return true;
    };
    globalThis.connectPeerDiagnostics = () => ({
        peer_bound: peerBound, event_bound_per_peer: 128, omitted_peers: omittedPeers,
        peers: entries.map(entry => {
            let current;
            try { current = entry.peer?sample(entry.peer):entry.current; } catch { current = {unavailable: true}; }
            return {peer_index:entry.peer_index,connection_id:entry.connection_id,live_owner_retained:entry.peer!==null,current, events: entry.events, omitted_events: entry.omitted_events, snapshot_errors: entry.snapshot_errors,omitted_media_rows:entry.omitted_media_rows,omitted_listeners:entry.omitted_listeners};
        }),
    });
    globalThis.connectPeerMediaDiagnostics = async () => Promise.all(entries.map(async(entry)=>{
        if(!entry.peer)return [];
        try{
        const report=await entry.peer.getStats(),rows=[];
        if(!entry.peer)return [];entry.omitted_media_rows=0;
        const fields=["packetsSent","packetsReceived","bytesSent","bytesReceived","framesEncoded","framesDecoded",
            "framesReceived","framesDropped","framesSent","keyFramesEncoded","keyFramesDecoded","frames","framesPerSecond","frameRate","totalEncodeTime","totalSamplesSent","totalSamplesReceived","totalAudioEnergy",
            "totalSamplesDuration","audioLevel","frameWidth","frameHeight"];
        fields.push("packetsLost","nackCount","pliCount","firCount","retransmittedPacketsSent",
            "retransmittedBytesSent","concealedSamples","silentConcealedSamples","concealmentEvents",
            "insertedSamplesForDeceleration","removedSamplesForAcceleration","packetsDiscarded",
            "jitterBufferDelay","jitterBufferEmittedCount","jitterBufferTargetDelay",
            "jitterBufferMinimumDelay","totalDecodeTime","totalProcessingDelay",
            "totalInterFrameDelay","totalSquaredInterFrameDelay","freezeCount","totalFreezesDuration");
        report.forEach((stat,key)=>{
            if(!["inbound-rtp","outbound-rtp","media-source"].includes(stat.type))return;
            if(rows.length>=8){entry.omitted_media_rows++;return;}
            let ordinal=entry.stats_ids.get(key);
            if(ordinal===undefined){
                if(typeof key!=='string'||key.length>256||entry.stats_ids.size>=32){entry.omitted_media_rows++;return;}
                ordinal=entry.stats_ids.size;entry.stats_ids.set(key,ordinal);
            }
            const row={peer_index:entry.peer_index,connection_id:entry.connection_id,report_index:ordinal,type:stat.type,
                kind:["audio","video"].includes(stat.kind)?stat.kind:"other",
                stats_timestamp_ms:Number.isFinite(stat.timestamp)?stat.timestamp:null};
            if(['none','cpu','bandwidth','other'].includes(stat.qualityLimitationReason))row.qualityLimitationReason=stat.qualityLimitationReason;
            for(const field of fields)if(Number.isFinite(stat[field])&&(stat[field]>=0||field==='packetsLost'))row[field]=stat[field];
            rows.push(row);
        });
        return rows;
        }catch{return [{peer_index:entry.peer_index,connection_id:entry.connection_id,unavailable:true}];}
    }));
    globalThis.connectPeerClockDiagnostics = async () => Promise.all(entries.map(async(entry)=>{
        try{
        if(!entry.peer)return [];
        const report=await entry.peer.getStats(),rows=[],now=performance.timeOrigin+performance.now();
        const numeric=value=>Number.isFinite(value)&&value>=0?value:null;
        const difference=(left,right)=>Number.isFinite(left)&&Number.isFinite(right)&&Math.abs(left-right)<=3600000
            ?left-right:null;
        report.forEach(stat=>{
            if(!['inbound-rtp','remote-outbound-rtp'].includes(stat.type)||rows.length>=8)return;
            const kind=stat.kind??report.get(stat.localId)?.kind;
            if(!['audio','video'].includes(kind))return;
            const row={type:stat.type,kind};
            if(stat.type==='remote-outbound-rtp'){
                row.reports_sent=numeric(stat.reportsSent);
                row.remote_timestamp_present=Number.isFinite(stat.remoteTimestamp)&&stat.remoteTimestamp>0;
                row.last_sr_received_age_ms=difference(now,stat.timestamp);
                // Remote and local clocks need not be synchronized. This is
                // a raw clock-domain difference, never a network-delay claim.
                row.local_minus_remote_clock_ms=row.remote_timestamp_present?difference(stat.timestamp,stat.remoteTimestamp):null;
            }else{
                row.estimated_playout_timestamp_present=Number.isFinite(stat.estimatedPlayoutTimestamp)&&stat.estimatedPlayoutTimestamp>0;
                row.estimated_playout_minus_stats_ms=row.estimated_playout_timestamp_present
                    ?difference(stat.estimatedPlayoutTimestamp,stat.timestamp):null;
                for(const key of ['packetsReceived','framesDecoded','totalSamplesReceived'])row[key]=numeric(stat[key]);
                // W3C receiver counters distinguish actual concealment and
                // adaptive playout from packet receipt. They are observations,
                // never an exception to decoded tone/AV acceptance.
                if(kind==='audio')for(const key of ['concealedSamples','silentConcealedSamples','concealmentEvents',
                    'insertedSamplesForDeceleration','removedSamplesForAcceleration','packetsDiscarded','packetsLost'])
                    row[key]=numeric(stat[key]);
                const emitted=numeric(stat.jitterBufferEmittedCount);
                row.jitterBufferEmittedCount=emitted;
                for(const [key,label]of [['jitterBufferDelay','jitter_buffer_mean_ms'],
                    ['jitterBufferTargetDelay','jitter_buffer_target_mean_ms'],
                    ['jitterBufferMinimumDelay','jitter_buffer_minimum_mean_ms']]){
                    row[key]=numeric(stat[key]);
                    row[label]=emitted>0&&numeric(stat[key])!==null?numeric(1000*stat[key]/emitted):null;
                }
            }
            rows.push(row);
        });
        return rows;
        }catch{return {unavailable:true};}
    }));
    const descriptionCodecs=description=>{
        if(!description||typeof description.sdp!=="string"||description.sdp.length>65536)return {unavailable:true};
        const media=[];let section;
        for(const line of description.sdp.split(/\r?\n/)){
            let match;
            if(line.startsWith("m=")){
                const kind=/^m=(audio|video) /.exec(line)?.[1];
                section=kind&&media.length<4?{kind,codecs:[],feedback:[]}:undefined;
                if(section)media.push(section);
            }else if(section&&(match=/^a=(sendrecv|sendonly|recvonly|inactive)$/.exec(line)))section.direction=match[1];
            else if(section&&section.codecs.length<32&&(match=/^a=rtpmap:(\d{1,3}) ([A-Za-z0-9._-]{1,32})\/(\d{1,8})(?:\/(\d{1,2}))?$/.exec(line)))
                section.codecs.push({payload_type:Number(match[1]),name:match[2],clock_rate:Number(match[3]),channels:match[4]===undefined?null:Number(match[4])});
            else if(section&&section.feedback.length<64&&(match=/^a=rtcp-fb:(\d{1,3}|\*) ([A-Za-z0-9_-]{1,32})(?: ([A-Za-z0-9_-]{1,32}))?$/.exec(line)))
                section.feedback.push({payload_type:match[1]==="*"?"*":Number(match[1]),type:match[2],parameter:match[3]??""});
        }
        return {type:["offer","answer","pranswer","rollback"].includes(description.type)?description.type:"other",media};
    };
    const codecSnapshot=peer=>({
        local:descriptionCodecs(peer.localDescription),remote:descriptionCodecs(peer.remoteDescription),
        transceivers:peer.getTransceivers().slice(0,4).map(tx=>({
            direction:state(tx.direction,["sendrecv","sendonly","recvonly","inactive","stopped"]),
            current_direction:tx.currentDirection===null?null:state(tx.currentDirection,["sendrecv","sendonly","recvonly","inactive","stopped"])}))});
    globalThis.connectPeerRetireCycle=()=>{
        const snapshot=globalThis.connectPeerDiagnostics();
        if(entries.some(entry=>entry.peer!==null||entry.detach!==null))
            return {released:false,reason:'original-peer-still-live',snapshot};
        entries.length=0;omittedPeers=0;
        return {released:true,retained_peer_owners:0,snapshot};
    };
    globalThis.connectPeerCodecDiagnostics=()=>entries.map(entry=>{
        if(!entry.peer)return {closed:true,...(entry.codecs??{unavailable:true})};
        try{return codecSnapshot(entry.peer);}catch{return {unavailable:true};}
    });
}

export async function browserFailure(pages, error) {
    const peerDiagnostics = await Promise.all(Array.from(pages, async (page, index) => {
        try {
            const value = await Promise.race([
                page.evaluate(() => globalThis.connectPeerDiagnostics?.() ?? {unavailable: true}),
                new Promise((_, reject) => setTimeout(() => reject(Error()), 1000)),
            ]);
            const control=await page.evaluate(()=>globalThis.connectControlDiagnostics?.()??{unavailable:true});
            const media=await Promise.race([page.evaluate(async()=>{
                const f=globalThis.videoFixture;
                if(!f?.session)return {unavailable:true};
                return {rtp:await globalThis.connectPeerMediaDiagnostics?.(),codecs:globalThis.connectPeerCodecDiagnostics?.(),video:(await f.session.diagnostics()).slice(0,4),
                    audio_context_state:f.audio?.state,source_draws:f.sourceObservation?.draws,
                    element:f.video?{ready_state:f.video.readyState,network_state:f.video.networkState,
                        media_error_code:f.video.error?.code??null,paused:f.video.paused,
                        width:f.video.videoWidth,height:f.video.videoHeight}:undefined,
                    remote_tracks:f.session.remoteStream.getTracks().slice(0,4).map(track=>({
                        kind:track.kind,ready_state:track.readyState,enabled:track.enabled,muted:track.muted}))};
            }),new Promise(resolve=>setTimeout(()=>resolve({unavailable:true}),1000))]);
            return {participant: index, ...value,control,media};
        } catch { return {participant: index, unavailable: true}; }
    }));
    return {status: "failed", profile: "connect-video-vp8/1", phase: "browser-participation",
        failure_code: [
            ['decoded-audio-readiness','Identifiable duplex decoded audio did not become ready within five seconds'],
            ['proxy-configuration','Explicit owned loopback media proxy range required'],
            ['proxy-path-bypassed','Actual media candidate pair bypassed its owned impairment proxy'],
            ['bridge-path-unproved','Owned internal bridge selected path not proved'],
        ].find(([,message])=>String(error?.message).includes(message))?.[0]??'other',
        error_class: ["Error", "TimeoutError", "ConnectError"].includes(error?.name) ? error.name : "OtherError",
        observations: [], peer_diagnostics: peerDiagnostics,
        owned_network_paths:error?.owned_network_paths,
        diagnostic_scope: "bounded peer state, numeric RTP counters and codec/direction metadata; no raw SDP, ICE candidates, keys, credentials or media content"};
}

/** Bounded wire correlation metadata; no payloads, signatures or addresses. */
export function installControlDiagnostics(){
    const NativeSocket=globalThis.WebSocket,frames=[];
    const record=(direction,body)=>{
        if(frames.length>=128||typeof body!=="string"||body.length>65536)return;
        try{const frame=JSON.parse(body);frames.push({direction,type:frame.type,id:frame.id,
            in_reply_to:frame.in_reply_to,cid:frame.cid,sid:frame.sid,connid:frame.connid,
            operation_id:frame.operation_id,expected_revision:frame.expected_revision});}catch{}
    };
    globalThis.WebSocket=class extends NativeSocket{
        constructor(...args){super(...args);this.addEventListener("message",event=>record("received",event.data));}
        send(...args){record("sent",args[0]);return super.send(...args);}
    };
    globalThis.connectControlDiagnostics=()=>({frame_bound:128,frames});
}
