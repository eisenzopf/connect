/** Fixture-only selected-path proof. Public observations contain no addresses. */
export function installNetworkProxyProof() {
    const NativePeer=globalThis.RTCPeerConnection,peers=[];
    globalThis.RTCPeerConnection=class extends NativePeer {
        constructor(...args){super(...args);peers.push(this);}
    };
    globalThis.connectOwnedMediaProxyProof=async({address,portStart,portEnd})=>{
        if(address!=='127.0.0.1'||!Number.isInteger(portStart)||!Number.isInteger(portEnd)
            ||portStart<1024||portEnd-portStart!==7||peers.length!==1)
            throw Error('Explicit owned loopback media proxy range required');
        const report=await peers[0].getStats(),pairs=[];
        report.forEach(stat=>{
            if(stat.type!=='transport'||!stat.selectedCandidatePairId)return;
            const pair=report.get(stat.selectedCandidatePairId),remote=report.get(pair?.remoteCandidateId);
            pairs.push({succeeded:pair?.state==='succeeded',nominated:pair?.nominated===true,
                udp:remote?.protocol==='udp',sent:pair?.bytesSent>0,received:pair?.bytesReceived>0,
                prescribed_address_selected:(remote?.address??remote?.ip)===address,
                prescribed_port_selected:Number.isInteger(remote?.port)&&remote.port>=portStart&&remote.port<=portEnd});
        });
        return {peer_count:peers.length,selected_pairs:pairs};
    };
}

export async function observedProxyPaths(pages,expected) {
    const until=performance.now()+2000;
    let paths=[];
    while(performance.now()<until){
        let timer;
        try{
            const observed=await Promise.race([
                Promise.all(pages.map(page=>page.evaluate(expected=>globalThis.connectOwnedMediaProxyProof(expected),expected))),
                new Promise(resolve=>{timer=setTimeout(()=>resolve(null),Math.max(0,until-performance.now()));}),
            ]);
            if(observed===null)break;
            paths=observed;
        }finally{clearTimeout(timer);}
        if(performance.now()<=until&&paths.length===2&&paths.every(row=>row.peer_count===1&&row.selected_pairs.length===1
            &&Object.values(row.selected_pairs[0]).every(value=>value===true)))return paths;
        // A selected nominated path can report an in-progress connectivity
        // check under loss/delay. Reobserve stats only; never replay an intent.
        await new Promise(resolve=>setTimeout(resolve,Math.min(25,Math.max(0,until-performance.now()))));
    }
    const error=Error('Actual media candidate pair bypassed its owned impairment proxy');
    error.owned_network_paths=paths;throw error;
}
