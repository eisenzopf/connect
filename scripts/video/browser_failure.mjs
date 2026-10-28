/** Bounded public peer state diagnostics. Never retain SDP, addresses or credentials. */
export function installPeerDiagnostics() {
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
            if (entries.length >= 4) { omittedPeers++; return; }
            const entry = {peer: this, events: [], omitted_events: 0, snapshot_errors: 0};
            entries.push(entry);
            const observedTransports = new WeakSet();
            const record = () => {
                try {
                    const value = {...sample(this), elapsed_ms: performance.now()};
                    if (entry.events.length < 128) entry.events.push(value);
                    else entry.omitted_events++;
                    for (const transceiver of this.getTransceivers()) {
                        for (const transport of [transceiver.sender?.transport, transceiver.receiver?.transport]) {
                            if (transport && !observedTransports.has(transport)) {
                                observedTransports.add(transport);
                                transport.addEventListener("statechange", record);
                            }
                        }
                    }
                } catch { entry.snapshot_errors++; }
            };
            for (const event of ["connectionstatechange", "iceconnectionstatechange", "icegatheringstatechange", "signalingstatechange"])
                this.addEventListener(event, record);
            record();
        }
    };
    globalThis.connectPeerDiagnostics = () => ({
        peer_bound: 4, event_bound_per_peer: 128, omitted_peers: omittedPeers,
        peers: entries.map(entry => {
            let current;
            try { current = sample(entry.peer); } catch { current = {unavailable: true}; }
            return {current, events: entry.events, omitted_events: entry.omitted_events, snapshot_errors: entry.snapshot_errors};
        }),
    });
    globalThis.connectPeerMediaDiagnostics = async () => Promise.all(entries.map(async(entry)=>{
        const report=await entry.peer.getStats(),rows=[];
        const fields=["packetsSent","packetsReceived","bytesSent","bytesReceived","framesEncoded","framesDecoded",
            "framesReceived","framesDropped","totalSamplesSent","totalSamplesReceived","totalAudioEnergy",
            "totalSamplesDuration","audioLevel","frameWidth","frameHeight"];
        report.forEach(stat=>{
            if(!["inbound-rtp","outbound-rtp","media-source"].includes(stat.type)||rows.length>=8)return;
            const row={type:stat.type,kind:["audio","video"].includes(stat.kind)?stat.kind:"other"};
            for(const field of fields)if(Number.isFinite(stat[field])&&stat[field]>=0)row[field]=stat[field];
            rows.push(row);
        });
        return rows;
    }));
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
                return {rtp:await globalThis.connectPeerMediaDiagnostics?.(),video:(await f.session.diagnostics()).slice(0,4),
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
        error_class: ["Error", "TimeoutError", "ConnectError"].includes(error?.name) ? error.name : "OtherError",
        observations: [], peer_diagnostics: peerDiagnostics,
        diagnostic_scope: "bounded native RTCPeerConnection/DTLS state events and description-presence flags; no SDP, ICE candidates, keys, credentials or media content"};
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
