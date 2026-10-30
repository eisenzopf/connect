/** Actual expired REST / unreachable TURN refusal; no private transport values in results. */
import {installPeerDiagnostics} from './browser_failure.mjs';

export async function refusedRelay(browser,spec,reason) {
    if(!['expired','unreachable'].includes(reason))throw Error('Unknown owned relay refusal scene');
    const results=[];
    for(const [index,configuration] of spec.participants.entries()) {
        const context=await browser.newContext();
        try {
            if(spec.browser==='chrome'&&process.platform==='linux'&&process.env.CONNECT_GATE6_OWNED_BROWSER_CONTAINER==='1'){
                await context.grantPermissions(['microphone','camera'],{origin:new URL(spec.baseURL).origin});
            }
            const page=await context.newPage();
            await page.addInitScript(installPeerDiagnostics);
            await page.goto(spec.baseURL+'/examples/conversation-client/');
            await page.mouse.click(1,1);
            results.push(await page.evaluate(async({configuration,index,reason})=>{
                const {BrowserVoiceSession}=await import('/sdk/typescript/dist/browser.js');
                const {canonicalMediaClient}=await import('/examples/conversation-client/canonical-media.js');
                const control=await canonicalMediaClient(configuration);
                const audio=new AudioContext();await audio.resume();
                const tone=audio.createOscillator(),destination=audio.createMediaStreamDestination();
                tone.connect(destination);tone.start();
                const stream=destination.stream,request=control.client.request.bind(control.client);
                let offers=0,opened,error,submitted;
                control.client.request=async(...args)=>{if(args[0]==='connection.offer')offers++;return request(...args);};
                const started=performance.now();
                try {
                    try {
                        opened=await BrowserVoiceSession.open(control.client,{...control.options,
                            videoStreamId:undefined,operationId:`browser-${reason}-relay-${index}`,inputStream:stream},{timeoutMs:8000});
                    }catch(value){error=value?.code;submitted=value?.savedOffer?.submissionAttempted??false;}
                    await opened?.close();
                    const peers=globalThis.connectPeerDiagnostics().peers;
                    const tracksEnded=stream.getTracks().every(track=>track.readyState==='ended');
                    if(opened||!['unsupported-media-profile','timeout'].includes(error)||offers||submitted||!tracksEnded
                        ||peers.length!==1||peers[0].current.connection!=='closed'||peers[0].current.remote_description_set)
                        throw Error('Relay refusal witness mismatch: '+JSON.stringify({opened:Boolean(opened),
                            error_code:['unsupported-media-profile','timeout','invalid-request'].includes(error)?error:'other',
                            logical_offers:offers,submitted,tracks_ended:tracksEnded,peers:peers.map(p=>p.current)}));
                    return {participant:index,error_code:error,seconds:(performance.now()-started)/1000,
                        limit_seconds:8,logical_offers:offers,submission_attempted:submitted,
                        owned_peer_closed:true,owned_audio_tracks_ended:tracksEnded,remote_description_set:false};
                } finally {
                    tone.stop();stream.getTracks().forEach(track=>track.stop());
                    await audio.close();control.client.close();
                }
            },{configuration,index,reason}));
        } finally {await context.close();}
    }
    return {status:'passed',reason,results,original_contexts_closed:results.length};
}
