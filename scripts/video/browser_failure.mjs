/** Bounded peer metadata. Default retains no addresses; explicit source-frame
 * witness copies only selected private fixture UDP endpoint/track facts. Never
 * retain raw SDP, credentials, keys, payloads or live track objects. */
export function installPeerDiagnostics({peerBound=4,rtcAudioFacts=false,rtcSourceFrameWitness=false}={}) {
    if(typeof rtcSourceFrameWitness!=='boolean')throw TypeError('Explicit source-frame witness boolean required');
    if(typeof rtcAudioFacts!=="boolean")throw TypeError("Explicit RTC audio diagnostic option must be boolean");
    // Runs in the existing original page init, before any observer/peer starts.
    Object.defineProperty(globalThis,"connectRtcAudioFacts",{value:rtcAudioFacts,configurable:true});
    if(!Number.isInteger(peerBound)||peerBound<1||peerBound>16)throw Error('Bounded original peer diagnostics');
    const NativePeer = globalThis.RTCPeerConnection;
    const entries = [];
    let omittedPeers = 0;
    let terminalCapture,terminalFinish,describeTagged;
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
            let terminal,returned=false;
            if(rtcSourceFrameWitness&&entry&&!entry.terminal_binding_started){
                entry.terminal_binding_started=true;
                try{terminal=terminalCapture(this,entry);}catch{terminal={profile:'connect/rtc-native-terminal-binding/1',peer_index:entry.peer_index,connection_id:entry.connection_id,captured_at_ms:null,captured_done_at_ms:null,native_close_observed:false,native_closed_at_ms:null,complete:false,reason:'native-terminal-unavailable',bindings:{}};}
            }
            try{const value=super.close();returned=true;return value;}
            finally{
                const closed=!!entry&&(this.connectionState==='closed'||this.signalingState==='closed');
                if(terminal)entry.terminal_binding=terminalFinish(terminal,closed,returned);
                if(entry&&closed){
                    try{entry.current=sample(this);}catch{entry.current={unavailable:true};}
                    entry.detach?.();entry.detach=null;entry.peer=null;entry.stats_ids.clear();
                    if(rtcSourceFrameWitness){entry.video_send_pointers=undefined;entry.video_receive_pointers=undefined;}
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
    // Opt-in private endpoint/track metadata from the SAME original peer.
    if(rtcSourceFrameWitness){
    const object=value=>value!==null&&(typeof value==='object'||typeof value==='function');
    globalThis.connectPeerStreamTag=(peer,connectionId,stream,direction,track)=>{
        try{
        const entry=entries.find(value=>value.peer===peer);
        if(!entry||entry.connection_id!==connectionId||!['send','receive'].includes(direction)||typeof stream!=='string'||stream.length<1||stream.length>256)return false;
        const txs=peer.getTransceivers();if(txs.length>4)return false;
        const key='video_'+direction;let descriptor,pointers;
        if(direction==='send'){
            const index=txs.indexOf(track),originalTrack=track?.sender?.track;
            if(index<0||!object(track.sender)||!originalTrack||originalTrack.kind!=='video'||typeof originalTrack.id!=='string'||originalTrack.id.length<1||originalTrack.id.length>256)return false;
            descriptor={connection_id:connectionId,stream_id:stream,track_id:originalTrack.id,transceiver_index:index};
            pointers={tx:new WeakSet([track]),endpoint:new WeakSet([track.sender]),track:new WeakSet([originalTrack])};
        }else{
            const selected=txs.filter(tx=>tx.receiver?.track===track);if(selected.length!==1||!track||track.kind!=='video'||typeof track.id!=='string'||track.id.length<1||track.id.length>256||!peer.getReceivers().some(value=>value===selected[0].receiver))return false;
            descriptor={connection_id:connectionId,stream_id:stream,track_id:track.id,transceiver_index:txs.indexOf(selected[0])};
            pointers={tx:new WeakSet(selected),endpoint:new WeakSet([selected[0].receiver]),track:new WeakSet([track])};
        }
        const actualTx=txs[descriptor.transceiver_index],actualEndpoint=direction==='send'?actualTx.sender:actualTx.receiver;
        if(entry[key])return JSON.stringify(entry[key])===JSON.stringify(descriptor)&&entry[key+'_pointers']?.tx.has(actualTx)===true&&entry[key+'_pointers']?.endpoint.has(actualEndpoint)===true&&entry[key+'_pointers']?.track.has(actualEndpoint.track)===true;
        entry[key]=descriptor;entry[key+'_pointers']=pointers;
        if(typeof actualTx.mid==='string'&&/^[A-Za-z0-9_-]{1,64}$/.test(actualTx.mid))entry[key+'_mid']=actualTx.mid;
        return true;
        }catch{return false;}
    };
    describeTagged=(entry,direction)=>{
        try{
        const peer=entry.peer,descriptor=entry['video_'+direction],pointers=entry['video_'+direction+'_pointers'];if(!peer||!descriptor||!pointers)return null;
        const txs=peer.getTransceivers();if(txs.length>4)return null;const tx=txs[descriptor.transceiver_index];
        if(!tx||!pointers.tx.has(tx)||typeof tx.mid!=='string'||!/^[A-Za-z0-9_-]{1,64}$/.test(tx.mid))return null;
        const endpoint=direction==='send'?tx.sender:tx.receiver,track=endpoint?.track;
        if(!object(endpoint)||!pointers.endpoint.has(endpoint)||!track||track.kind!=='video'||typeof track.id!=='string'||track.id.length<1||track.id.length>256)return null;
        if(direction==='receive'&&(track.id!==descriptor.track_id||!pointers.track.has(track)||!peer.getReceivers().includes(endpoint)))return null;
        if(direction==='send'){
            if(descriptor.track_id!==track.id||!pointers.track.has(track))return null;
        }
        const midKey='video_'+direction+'_mid';
        if(entry[midKey]!==undefined&&entry[midKey]!==tx.mid)return null;entry[midKey]=tx.mid;
        return {connection_id:entry.connection_id,stream_id:descriptor.stream_id,track_id:track.id,transceiver_index:descriptor.transceiver_index,mid:tx.mid};
        }catch{return null;}
    };
    globalThis.connectPeerStreamDescriptor=(connectionId,direction)=>{
        const found=entries.filter(value=>value.peer&&value.connection_id===connectionId);return found.length===1?describeTagged(found[0],direction):null;
    };
    }
    globalThis.connectPeerDiagnostics = () => ({
        peer_bound: peerBound, event_bound_per_peer: 128, omitted_peers: omittedPeers,
        peers: entries.map(entry => {
            let current;
            try { current = entry.peer?sample(entry.peer):entry.current; } catch { current = {unavailable: true}; }
            return {peer_index:entry.peer_index,connection_id:entry.connection_id,live_owner_retained:entry.peer!==null,current, events: entry.events, omitted_events: entry.omitted_events, snapshot_errors: entry.snapshot_errors,omitted_media_rows:entry.omitted_media_rows,omitted_listeners:entry.omitted_listeners,...(rtcSourceFrameWitness?{terminal_binding:entry.terminal_binding??null,public_binding:entry.public_binding??null}:{})};
        }),
    });
    globalThis.connectPeerMediaDiagnostics = async () => Promise.all(entries.map(async(entry)=>{
        if(!entry.peer)return [];
        try{
        const report=await entry.peer.getStats(),rows=[];
        if(!entry.peer)return [];entry.omitted_media_rows=0;
        const fields=["ssrc","packetsSent","packetsReceived","bytesSent","bytesReceived","framesEncoded","framesDecoded",
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
            if(rtcSourceFrameWitness&&stat.kind==='video'&&['outbound-rtp','inbound-rtp'].includes(stat.type)){
                const direction=stat.type==='outbound-rtp'?'send':'receive';
                row.original_track=globalThis.connectPeerStreamDescriptor(entry.connection_id,direction);
                const codec=report.get(stat.codecId),transport=report.get(stat.transportId),pair=report.get(transport?.selectedCandidatePairId),
                    local=report.get(pair?.localCandidateId),remote=report.get(pair?.remoteCandidateId);
                row.codec={mime_type:typeof codec?.mimeType==='string'?codec.mimeType:null,
                    payload_type:Number.isInteger(codec?.payloadType)?codec.payloadType:null,
                    clock_rate:Number.isInteger(codec?.clockRate)?codec.clockRate:null};
                const endpoint=value=>value&&typeof value.address==='string'&&value.address.length<=128
                    &&Number.isInteger(value.port)&&value.port>0&&value.port<=65535?{address:value.address,port:value.port,protocol:value.protocol}:null;
                row.selected_path={nominated:pair?.nominated===true,succeeded:pair?.state==='succeeded',
                    local:endpoint(local),remote:endpoint(remote),srtp_cipher:typeof transport?.srtpCipher==='string'?transport.srtpCipher:null};
            }
            if(stat.type==='inbound-rtp'&&stat.kind==='audio'){
                // Same original poll, owner and native report identity. Missing
                // or invalid data stays unavailable; never manufacture zero.
                const counters=['ssrc','concealedSamples','silentConcealedSamples','concealmentEvents',
                    'insertedSamplesForDeceleration','removedSamplesForAcceleration','packetsLost','packetsDiscarded',
                    'jitterBufferDelay','jitterBufferEmittedCount','jitterBufferTargetDelay','jitterBufferMinimumDelay',
                    'totalSamplesReceived','totalSamplesDuration'];
                row.audio_counter_provenance=Object.freeze(Object.fromEntries(counters.map(field=>{
                    const value=stat[field],available=Number.isFinite(value)&&(value>=0||field==='packetsLost');
                    return [field,Object.freeze({present:field in stat,available,value:available?value:null})];
                })));
            }
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
    // All helpers remain inside this serialized page-init function. The
    // explicit witness option alone owns these scalar terminal observations.
    if(rtcSourceFrameWitness){
        const boundedId=value=>typeof value==='string'&&value.length>0&&value.length<=256&&!/[\x00-\x1f\x7f]/.test(value);
        const u32=value=>Number.isInteger(value)&&value>=0&&value<=0xffffffff;
        const capturedReasons=new WeakMap(),nativePathDetails=new WeakMap();
        const fail=(reason,detail)=>{const marker={};capturedReasons.set(marker,reason);if(detail)nativePathDetails.set(marker,detail);throw marker;};
        const reason=(error,fallback)=>{try{return capturedReasons.get(error)??fallback;}catch{return fallback;}};
        const freeze=value=>{if(value&&typeof value==='object'){for(const child of Object.values(value))freeze(child);Object.freeze(value);}return value;};
        const numericAddress=value=>{
            if(typeof value!=='string'||value.length<1||value.length>128)return false;
            if(value.includes(':')){if(!/^[0-9a-fA-F:]+$/.test(value))return false;try{new URL('http://['+value+']/');return true;}catch{return false;}}
            return /^(?:[0-9]{1,3}\.){3}[0-9]{1,3}$/.test(value)&&value.split('.').every(part=>Number(part)<=255&&String(Number(part))===part);
        };
        const sections=description=>{
            if(!description||!['offer','answer','pranswer'].includes(description.type)||typeof description.sdp!=='string'||description.sdp.length>65536)fail('sdp-unavailable');
            const lines=description.sdp.split(/\r?\n/);if(lines.length>2048||lines.some(line=>line.length>1024))fail('sdp-bound');
            const result=[];let current;
            for(const line of lines){if(line.startsWith('m=')){if(result.length>=4)fail('media-section-bound');current={kind:line.split(' ')[0].slice(2),lines:[]};result.push(current);}if(current)current.lines.push(line);}
            return result;
        };
        const videoSection=(description,mid)=>{
            const all=sections(description),mids=[];
            for(const section of all){const values=section.lines.filter(line=>line.startsWith('a=mid:'));if(values.length>1)fail('mid-unavailable-or-ambiguous');for(const value of values){if(mids.includes(value))fail('mid-unavailable-or-ambiguous');mids.push(value);}}
            const found=all.filter(section=>section.kind==='video'&&section.lines.includes('a=mid:'+mid));
            if(found.length!==1)fail('mid-unavailable-or-ambiguous');
            const selected=found[0];if(selected.lines.filter(line=>line.startsWith('a=mid:')).length!==1)fail('mid-unavailable-or-ambiguous');
            const groups=selected.lines.filter(line=>line.startsWith('a=ssrc-group:'));
            if(groups.length>1||groups.some(line=>!/^a=ssrc-group:FID [0-9]{1,10} [0-9]{1,10}$/.test(line)))fail('source-ssrc-group-unsupported');return selected;
        };
        const declarations=section=>{
            const result=[],seen=new Set(),formats=section.lines[0].split(/ +/).slice(3);
            if(!formats.length||formats.length>32||formats.some(value=>!/^\d{1,3}$/.test(value)||Number(value)>=128)||new Set(formats).size!==formats.length)fail('codec-declaration-invalid');
            for(const line of section.lines){if(!line.startsWith('a=rtpmap:'))continue;
                const match=/^a=rtpmap:([0-9]{1,3}) ([A-Za-z0-9._-]{1,32})\/([0-9]{1,8})$/.exec(line);
                if(!match||result.length>=32)fail('codec-declaration-invalid');
                const pt=Number(match[1]),clock=Number(match[3]);if(pt>=128||clock<1||seen.has(pt)||!formats.includes(String(pt)))fail('codec-declaration-invalid');seen.add(pt);
                result.push({payload_type:pt,mime_type:'video/'+match[2],clock_rate:clock});
            }
            if(!result.some(value=>value.mime_type.toLowerCase()==='video/vp8'&&value.clock_rate===90000))fail('codec-declaration-invalid');return result;
        };
        const senderSsrc=section=>{
            const ids=new Set(),fid=[];
            for(const line of section.lines){
                if(line.startsWith('a=ssrc-group:')){
                    const match=/^a=ssrc-group:FID ([0-9]{1,10}) ([0-9]{1,10})$/.exec(line);
                    if(!match||fid.length>=1)fail('source-ssrc-group-unsupported');
                    const values=match.slice(1).map(Number);if(values.some(value=>!u32(value)||value===0)||values[0]===values[1])fail('source-ssrc-group-unsupported');fid.push(values);
                }else if(line.startsWith('a=ssrc:')){
                    const match=/^a=ssrc:([0-9]{1,10}) /.exec(line);if(!match)fail('source-ssrc-invalid');
                    const value=Number(match[1]);if(!u32(value)||value===0)fail('source-ssrc-invalid');ids.add(value);if(ids.size>2)fail('source-ssrc-ambiguous');
                }
            }
            if(fid.length){const [primary,rtx]=fid[0];if(ids.size!==2||!ids.has(primary)||!ids.has(rtx))fail('source-ssrc-ambiguous');return {primary_ssrc:primary,rtx_ssrc:rtx};}
            if(ids.size!==1)fail('source-ssrc-ambiguous');return {primary_ssrc:[...ids][0]};
        };
        // Failure-only actual native candidate facts, never a selected-path substitute.
        const candidateCensus=description=>{
            try{
                const rows=[],all=sections(description),seen=new Set();let total=0,omitted=0,invalid=0;
                for(const section of all){
                    const mids=section.lines.filter(line=>line.startsWith('a=mid:')).map(line=>line.slice(6));
                    const mid=mids.length===1&&boundedId(mids[0])&&!seen.has(mids[0])?mids[0]:null;
                    if(mid!==null)seen.add(mid);else invalid++;
                    for(const line of section.lines){if(!line.startsWith('a=candidate:'))continue;total++;
                        const fields=line.slice(12).split(/ +/),component=Number(fields[1]),port=Number(fields[5]),protocol=fields[2]?.toLowerCase(),type=fields[7];
                        if(mid===null||fields.length<8||!['1','2'].includes(fields[1])||!['udp','tcp'].includes(protocol)
                            ||!numericAddress(fields[4])||!/^\d{1,5}$/.test(fields[5])||port<1||port>65535||fields[6]!=='typ'
                            ||!['host','srflx','prflx','relay'].includes(type)){invalid++;continue;}
                        if(rows.length>=32){omitted++;continue;}
                        rows.push({mid,component,address:fields[4],port,protocol,type});
                    }
                }
                return {profile:'connect/rtc-remote-description-candidate-census/1',complete:invalid===0&&omitted===0,
                    reason:invalid?'candidate-census-incomplete':omitted?'candidate-census-bound':null,
                    candidate_lines:total,omitted,invalid,candidates:rows};
            }catch{return {profile:'connect/rtc-remote-description-candidate-census/1',complete:false,reason:'remote-description-unavailable',candidates:[]};}
        };
        const path=(transport,remoteDescription)=>{
            if(!transport||transport.state!=='connected')fail('dtls-unavailable');
            const dtlsState=transport.state,ice=transport.iceTransport;
            const iceState=ice?.state,iceReady=!!ice&&['connected','completed'].includes(iceState);
            let method,methodRead=false,pairRead=false,pair,local,remote;
            if(iceReady){method=ice.getSelectedCandidatePair;methodRead=true;}
            // Failure-only scalar observation of this SAME getter invocation.
            // No candidate text, hostname, SDP, credentials or exception string.
            const kind=value=>value===null?'null':typeof value;
            const address=value=>{
                if(value===undefined||value===null)return {address_kind:'absent'};
                if(typeof value!=='string'||value.length>128)return {address_kind:'other'};
                if(numericAddress(value))return {address_kind:value.includes(':')?'numeric-v6':'numeric-v4',address:value};
                // Diagnose a valid embedded-IPv4 IPv6 literal without accepting
                // it into the existing stricter path predicate or normalizing it.
                if(value.includes(':')&&/^[0-9a-fA-F:.]+$/.test(value)){
                    try{new URL('http://['+value+']/');return {address_kind:'numeric-v6',address:value};}catch{}
                }
                return {address_kind:/^[A-Za-z0-9_-]+\.local\.?$/.test(value)?'mdns':'other'};
            };
            const endpointShape=value=>{
                if(!value)return {read:false};
                if(!value.present)return {read:true,present:false};
                const portValid=value.port_read&&Number.isInteger(value.port)&&value.port>0&&value.port<=65535;
                return {read:true,present:true,...(value.address_read?address(value.address):{address_kind:'not-read'}),
                    port_kind:!value.port_read?'not-read':portValid?'valid':value.port===undefined||value.port===null?'absent':'invalid',
                    ...(portValid?{port:value.port}:{}),protocol_kind:!value.protocol_read?'not-read':['udp','tcp'].includes(value.protocol)?value.protocol:
                        value.protocol===undefined||value.protocol===null?'absent':'other'};
            };
            const remoteExtra=value=>{
                if(!value?.native)return {};
                const read=name=>{try{return {value:value.native[name],threw:false};}catch{return {threw:true};}};
                const type=read('type'),candidate=read('candidate'),port=value.port_read?{value:value.port}:read('port'),protocol=value.protocol_read?{value:value.protocol}:read('protocol');
                const validPort=!port.threw&&Number.isInteger(port.value)&&port.value>0&&port.value<=65535;
                return {candidate_type:type.threw?'threw':['host','srflx','prflx','relay'].includes(type.value)?type.value:type.value==null?'absent':'other',
                    candidate_kind:candidate.threw?'threw':typeof candidate.value==='string'?'string':candidate.value==null?'absent':'other',
                    candidate_empty:!candidate.threw&&typeof candidate.value==='string'?candidate.value.length===0:null,
                    port_kind:port.threw?'threw':validPort?'valid':port.value==null?'absent':'invalid',...(validPort?{port:port.value}:{}),
                    protocol_kind:protocol.threw?'threw':['udp','tcp'].includes(protocol.value)?protocol.value:protocol.value==null?'absent':'other'};
            };
            const detail=()=>({profile:'connect/rtc-native-selected-pair-shape/1',
                ice_state:['new','checking','connected','completed','disconnected','failed','closed'].includes(iceState)?iceState:
                    iceState===undefined||iceState===null?'absent':'other',
                dtls_state:dtlsState,method_kind:methodRead?kind(method):'not-read',pair_read:pairRead,pair_present:pairRead?pair!==null&&pair!==undefined:null,
                pair_kind:pairRead?kind(pair):'not-read',local:endpointShape(local),remote:{...endpointShape(remote),...remoteExtra(remote)},
                remote_description_candidates:candidateCensus(remoteDescription)});
            const unavailable=()=>fail('selected-pair-unavailable',detail());
            if(!iceReady||typeof method!=='function')unavailable();
            pairRead=true;pair=method.call(ice); // Exactly the existing one native API call.
            if(!pair)unavailable();
            // Preserve the original short-circuit property order. Failed local
            // fields do not cause extra remote/port/protocol getter reads.
            const endpoint=(value,side)=>{
                const observed={present:!!value,native:value};if(side==='local')local=observed;else remote=observed;
                if(!value)unavailable();
                observed.address=value.address;observed.address_read=true;
                if(!numericAddress(observed.address))unavailable();
                observed.port=value.port;observed.port_read=true;
                if(!Number.isInteger(observed.port)||observed.port<1||observed.port>65535)unavailable();
                observed.protocol=value.protocol;observed.protocol_read=true;
                if(observed.protocol!=='udp')unavailable();
                return {address:observed.address,port:observed.port,protocol:observed.protocol};
            };
            const selectedLocal=endpoint(pair.local,'local'),selectedRemote=endpoint(pair.remote,'remote');
            if(ice.state!==iceState||transport.state!==dtlsState)fail('selected-pair-changed');
            return {local:selectedLocal,remote:selectedRemote,ice_state:iceState,dtls_state:dtlsState,basis:'RTCIceTransport.getSelectedCandidatePair'};
        };
        const role=(peer,entry,direction)=>{
            try{
                const original=describeTagged(entry,direction);if(!original)fail('original-track-unavailable');
                const txs=peer.getTransceivers();if(txs.length>4)fail('transceiver-bound');const tx=txs[original.transceiver_index];
                if(!tx||tx.mid!==original.mid)fail('original-transceiver-changed');
                const track=direction==='send'?tx.sender?.track:tx.receiver?.track;
                if(!track||!['live','ended'].includes(track.readyState))fail('track-state-unavailable');
                const localDescription=peer.localDescription,remoteDescription=peer.remoteDescription;
                const local=videoSection(localDescription,original.mid),remote=videoSection(remoteDescription,original.mid);
                const value={complete:true,reason:null,original_track:original,track_state:track.readyState,
                    selected_udp_pair:path(direction==='send'?tx.sender?.transport:tx.receiver?.transport,remoteDescription),
                    codec_declarations:{local:declarations(local),remote:declarations(remote)}};
                if(direction==='send')Object.assign(value,senderSsrc(local));
                else{
                    if(typeof tx.receiver?.getSynchronizationSources!=='function')fail('synchronization-source-unavailable');
                    const sources=tx.receiver.getSynchronizationSources();if(!Array.isArray(sources)||sources.length!==1)fail('synchronization-source-ambiguous');
                    const source=sources[0];if(!source||!u32(source.source)||source.source===0||!u32(source.rtpTimestamp)||!Number.isFinite(source.timestamp)||source.timestamp<=0)fail('synchronization-source-invalid');
                    if(remote.lines.some(line=>line.startsWith('a=ssrc:')||line.startsWith('a=ssrc-group:'))&&senderSsrc(remote).primary_ssrc!==source.source)fail('synchronization-source-contradiction');
                    value.sync_source={source:source.source,rtpTimestamp:source.rtpTimestamp,timestamp:source.timestamp,timestamp_clock:'native-uninterpreted'};
                }
                return freeze(value);
            }catch(error){const detail=nativePathDetails.get(error);return freeze({complete:false,reason:reason(error,'native-binding-unavailable'),
                ...(detail?{native_path_diagnostic:detail}:{})});}
        };
        const bindingCapture=(peer,entry,profile)=>{
            const began=performance.now();let value={profile,peer_index:entry.peer_index,connection_id:entry.connection_id,
                captured_at_ms:Number.isFinite(began)&&began>=0?began:null,captured_done_at_ms:null,complete:false,reason:'native-terminal-unavailable',bindings:{}};
            try{
                if(!boundedId(entry.connection_id)||entry.peer!==peer||peer.connectionState!=='connected'||!['connected','completed'].includes(peer.iceConnectionState)||peer.signalingState!=='stable')fail('native-peer-unavailable');
                value.bindings={send:role(peer,entry,'send'),receive:role(peer,entry,'receive')};
                const done=performance.now();if(!Number.isFinite(began)||began<0||!Number.isFinite(done)||done<began)fail('native-clock-unavailable');
                value.captured_done_at_ms=done;value.complete=true;value.reason=null;
            }catch(error){value.complete=false;value.reason=reason(error,'native-terminal-unavailable');value.bindings={};}
            return value;
        };
        const freezeRecord=value=>{
            try{if(new TextEncoder().encode(JSON.stringify(value)).byteLength>32768)value={...value,complete:false,reason:'binding-metadata-bound',bindings:{}};}
            catch{value={...value,complete:false,reason:'native-binding-unavailable',bindings:{}};}
            return freeze(value);
        };
        terminalCapture=(peer,entry)=>bindingCapture(peer,entry,'connect/rtc-native-terminal-binding/1');
        terminalFinish=(value,closed,returned)=>{
            let at=null;try{const now=performance.now();if(Number.isFinite(now)&&now>=value.captured_done_at_ms)at=now;}catch{}
            return freezeRecord({...value,native_close_observed:closed,native_closed_at_ms:closed?at:null,
                complete:value.complete&&closed&&returned&&at!==null,
                reason:!returned?'native-close-error':!closed?'native-close-unconfirmed':at===null?'native-clock-unavailable':value.reason});
        };
        // The observer's immutable validated-cut ledger binds this ONE capture
        // to its operation/route/previous+target owner. It is never a new grant.
        globalThis.connectPeerPublicBinding=connectionId=>{
            if(!boundedId(connectionId))return null;
            const found=entries.filter(entry=>entry.connection_id===connectionId);if(found.length!==1)return null;
            const entry=found[0];if(entry.public_binding!==undefined)return entry.public_binding;
            if(!entry.peer)return null;
            try{entry.public_binding=freezeRecord(bindingCapture(entry.peer,entry,'connect/rtc-native-public-binding/1'));}
            catch{entry.public_binding=freezeRecord({profile:'connect/rtc-native-public-binding/1',peer_index:entry.peer_index,connection_id:entry.connection_id,captured_at_ms:null,captured_done_at_ms:null,complete:false,reason:'native-binding-unavailable',bindings:{}});}
            return entry.public_binding;
        };
    }

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
