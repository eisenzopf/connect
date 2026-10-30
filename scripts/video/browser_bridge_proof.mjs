/** Selected path proof for the explicitly owned internal Docker bridge. */
export function installBridgeObserver(){
    const NativePeer=globalThis.RTCPeerConnection,peers=[];
    globalThis.RTCPeerConnection=class extends NativePeer{
        constructor(...args){super(...args);if(peers.length>=4)throw Error('Owned bridge peer bound');peers.push(this);}
    };
    globalThis.connectOwnedBridgePaths=async expected=>Promise.all(peers.map(async peer=>{
        const stats=await peer.getStats(),paths=[];
        stats.forEach(transport=>{
            if(transport.type!=='transport'||!transport.selectedCandidatePairId)return;
            const pair=stats.get(transport.selectedCandidatePairId),remote=stats.get(pair?.remoteCandidateId);
            const local=stats.get(pair?.localCandidateId);
            paths.push({nominated:pair?.nominated===true,succeeded:pair?.state==='succeeded',
                expected_address:remote?.address===expected.ip,
                expected_port:Number.isInteger(remote?.port)&&remote.port>=expected.port_start&&remote.port<=expected.port_end,
                udp:remote?.protocol==='udp',sent:pair?.bytesSent>0,received:pair?.bytesReceived>0,
                ...(expected.require_relay?{local_relay:local?.candidateType==='relay',remote_relay:remote?.candidateType==='relay'}:{})});
        });
        return paths;
    }));
}

export async function observedBridgePaths(pages,expected){
    const paths=await Promise.all(pages.map(page=>page.evaluate(expected=>globalThis.connectOwnedBridgePaths(expected),expected)));
    if(paths.length!==2||paths.some(peers=>peers.length!==1||peers[0].length!==1
        ||Object.values(peers[0][0]).some(value=>value!==true)))throw Error('Owned internal bridge selected path not proved');
    return {participants:2,selected_paths:2,expected_private_gateway:!expected.require_relay,
        expected_private_relay:Boolean(expected.require_relay),expected_port_range:true,
        nominated_udp_duplex:true,observation_source:'RTCPeerConnection.getStats'};
}
