import { ReceiverVideoReleaseOwner, type ReceiverVideoReleaseLoan, type ReceiverVideoRelease } from "./browser_receiver_video_release.js";
import { bridgeCanonical, parseWebRtcReceiverPostCut, parseWebRtcPreCutKeyframes, receiverScope, type WebRtcReceiverPostCut, type WebRtcBridgeReadyMarker } from "./browser_webrtc_public_video_bridge.js";
import { BrowserDeadline } from "./browser_deadline.js";
import type { BrowserReplacementReceipt } from "./browser_replacement.js";
import { ConnectError } from "./errors.js";
import { jsonObject, type JsonObject } from "./uctp_json.js";
import { BrowserWebRtcReplacement, recoverableReadinessFailure, type WebRtcDecodedReadiness } from "./browser_webrtc_replacement.js";
import { WebRtcPublicVideoPhaseOwner, type WebRtcPublicVideoPhase } from "./browser_webrtc_public_video_phase.js";
import { observeWebRtcPublicVideoReadiness, prearmWebRtcPublicVideoBridge, type WebRtcPreCutPublicVideo, type WebRtcPreparedPublicVideo } from "./browser_webrtc_public_video_readiness.js";
import { WebRtcReceiveRoles } from "./browser_webrtc_receive_roles.js";
import type { WebRtcReceiveReservationScope } from "./browser_webrtc_receive_reservation.js";
import type { WebRtcPreparedVideo } from "./browser_webrtc_readiness.js";

type TrackTask = <T>(work: Promise<T>, cleanup?: boolean) => Promise<T>;
const invalid = () => new ConnectError("protocol", "Original reserved public video workflow changed");
const cancelled = () => new ConnectError("cancelled", "Original reserved public video wait cancelled");

/** The same original warmed element is loaned to one renderer at a time. A
 * renderer close may pause it, but cannot detach/reset its decoder or replace
 * the original B track. Connection retirement owns its final release. */
export interface WebRtcPublicVideoLoan extends WebRtcPreparedVideo {
    eligible(metadata:VideoFrameCallbackMetadata):boolean;
    presented(value: number,rtpTimestamp?:number): void;
    release(): void;
}
class CompletedPublicVideo {
    private picture?: WebRtcPreparedVideo;
    private track?: MediaStreamTrack;
    private loan?: object;
    private floor: number;
    constructor(picture: WebRtcPreparedVideo, track: MediaStreamTrack,private releaseOwner?:ReceiverVideoReleaseOwner) {
        this.picture=picture;this.track=track;this.floor=picture.presentedFloor;
    }
    borrow(track: MediaStreamTrack): WebRtcPublicVideoLoan {
        if (!this.picture || !this.track || this.track!==track || track.readyState!=="live" || this.loan) throw invalid();
        const picture=this.picture,token={};const released=this.releaseOwner?.borrow();this.loan=token;
        return Object.freeze({element:picture.element,presentedFloor:this.floor,
            eligible:(metadata:VideoFrameCallbackMetadata)=>this.loan===token&&(!released||released.eligibleRtp({rtpTimestamp:(metadata as VideoFrameCallbackMetadata&{rtpTimestamp?:number}).rtpTimestamp!,presentedFrames:metadata.presentedFrames})),
            presented:(value:number,rtpTimestamp?:number)=>{
                if (this.loan!==token || !Number.isSafeInteger(value) || value<=this.floor) throw invalid();
                released?.presentedRtp({rtpTimestamp:rtpTimestamp!,presentedFrames:value});this.floor=value;
            },
            release:()=>{if(this.loan===token){released?.release();this.loan=undefined;}},
        });
    }
    release(): Promise<void> {
        const picture=this.picture;this.picture=undefined;this.track=undefined;this.loan=undefined;this.releaseOwner?.close();this.releaseOwner=undefined;
        if (!picture) return Promise.resolve();
        let failure:unknown;
        try{picture.element.pause();}catch(error){failure=error;}
        try{picture.element.srcObject=null;}catch(error){failure??=error;}
        return failure===undefined?Promise.resolve():Promise.reject(failure);
    }
}

/** One original reserved B workflow. Verified operation.get observations are
 * supplied by the same signaling owner; plain caller phase/options never enter
 * this state machine. It creates no server authority, source cut or media grant. */
export class BrowserWebRtcPublicVideoLifecycle {
    private readonly phaseOwner: WebRtcPublicVideoPhaseOwner;
    private readonly cancel = new AbortController();
    private readonly signal: AbortSignal;
    private running?: Promise<BrowserReplacementReceipt>;
    private observation?: WebRtcPreparedPublicVideo;
    private prearmed?:WebRtcPreCutPublicVideo;
    private provisional?:Readonly<WebRtcReceiverPostCut>;
    private completed?: CompletedPublicVideo;
    private stopped=false;
    constructor(private readonly original: BrowserWebRtcReplacement,
        private peer: RTCPeerConnection | undefined,
        private roles: WebRtcReceiveRoles | undefined,
        scope: WebRtcReceiveReservationScope,
        private readonly budget: BrowserDeadline,
        signal: AbortSignal,
        private readonly trackTask: TrackTask) {
        const context=original.reservedContext();
        this.phaseOwner=new WebRtcPublicVideoPhaseOwner(context.root,scope,context.reservation,
            context.challenge.target_generation,context.challenge.nonce,context.bridgeOptIn);
        this.signal=AbortSignal.any([signal,this.cancel.signal]);
    }
    private check(): void {
        if (this.stopped || this.signal.aborted) throw cancelled();
        this.budget.remaining();this.phaseOwner.check();
        if (!this.peer || !this.roles || !this.phaseOwner.exactLocalPeer(this.peer) ||
            ["closed","failed"].includes(this.peer.connectionState)) throw invalid();
    }
    private options() {this.check();return {timeoutMs:Math.min(1000,Math.max(1,Math.floor(this.budget.remaining()/2))),signal:this.signal};}
    private run<T>(work:()=>Promise<T>):Promise<T> {
        this.check();return this.budget.run(()=>this.trackTask(work()),this.signal);
    }
    private async delay():Promise<void> {
        await this.run(()=>new Promise<void>(resolve=>setTimeout(resolve,Math.min(20,this.budget.remaining()))));
    }
    private async read():Promise<JsonObject> {
        const value=await this.run(()=>this.original.reservedStatus(this.options()));this.check();
        if (["failed","cancelled"].includes(String(value.state))) throw new ConnectError("refused","Original reserved replacement did not complete");
        if (!["accepted","preparing","ready","committing","unknown","interrupted","completed"].includes(String(value.state))) throw invalid();
        return value;
    }
    private phase(value:JsonObject,required:boolean):Readonly<WebRtcPublicVideoPhase> | undefined {
        const raw=value.webrtc_public_video_phase;
        if (raw===undefined) {if(required||value.state==="completed"||value.receiver_video_pre_cut_keyframes!==undefined)throw invalid();return undefined;}
        if (!jsonObject(raw)) throw invalid();
        const receipt=value.state==="completed"?this.original.reservedCompleted(value):undefined;
        const context=this.original.reservedContext();
        const phase=this.phaseOwner.observe(raw,{recordId:context.root.recordId,operationId:context.root.operationId,
            targetConnectionId:context.root.targetConnectionId,targetGeneration:context.challenge.target_generation,rootCompleted:receipt!==undefined});
        if(this.phaseOwner.bridgeEnabled())this.phaseOwner.observeBridge(value.webrtc_public_video_bridge);
        if(value.receiver_video_pre_cut_keyframes!==undefined) {
            if(!this.phaseOwner.bridgeEnabled()||value.state!=="preparing"||phase.phase!=="negotiated-never-fed"||
                !this.phaseOwner.observeBridge(value.webrtc_public_video_bridge).feeding)throw invalid();
            parseWebRtcPreCutKeyframes(value.receiver_video_pre_cut_keyframes);
        }
        if (phase.phase==="discarded") throw new ConnectError("refused","Original reserved public video was discarded");
        return phase;
    }
    complete(observePrivate:()=>Promise<WebRtcDecodedReadiness>,privateCurrent:()=>boolean):Promise<BrowserReplacementReceipt> {
        if(this.running)return this.running;
        let resolve!:(value:BrowserReplacementReceipt)=>void,reject!:(error:unknown)=>void;
        const promise=new Promise<BrowserReplacementReceipt>((yes,no)=>{resolve=yes;reject=no;});this.running=promise;
        void this.perform(observePrivate,privateCurrent).then(resolve,reject);return promise;
    }
    private async perform(observePrivate:()=>Promise<WebRtcDecodedReadiness>,privateCurrent:()=>boolean):Promise<BrowserReplacementReceipt> {
        let adopted=false,terminal:unknown;
        try{
            let current:JsonObject,phase:Readonly<WebRtcPublicVideoPhase>|undefined;
            for(;;){
                try{current=await this.read();}catch(error){if(!recoverableReadinessFailure(error))throw error;await this.delay();continue;}
                phase=this.phase(current,false);
                if(phase){if(phase.phase!=="negotiated-never-fed")throw invalid();break;}
                await this.delay();
            }
            if(this.phaseOwner.bridgeEnabled())this.prearmed=await prearmWebRtcPublicVideoBridge(this.phaseOwner,this.peer!,this.roles!,this.budget,this.signal,this.trackTask);
            this.prearmed?.assertPreCutCurrent();
            const evidence=await this.run(observePrivate);this.check();this.prearmed?.assertPreCutCurrent();
            if(!privateCurrent())throw invalid();
            let attempts=0,replay=false,next=0;
            const sendPrivate=async(original?:JsonObject)=>{
                this.check();if(!privateCurrent())throw invalid();attempts++;
                try{await this.run(()=>this.original.reservedPrivateReady(evidence,this.options(),original));replay=false;}
                catch(error){if(!recoverableReadinessFailure(error))throw error;replay=true;next=performance.now()+Math.min(1000,50*2**(attempts-1));}
            };
            await sendPrivate();
            let bridgeMarker:Readonly<WebRtcBridgeReadyMarker>|undefined,bridgeAttempts=0,bridgeReplay=false,bridgeNext=0;
            const sendBridge=async(original?:JsonObject)=>{
                this.check();bridgeAttempts++;
                try{await this.run(()=>this.original.reservedBridgeReady(bridgeMarker!,this.options(),original));bridgeReplay=false;}
                catch(error){if(!recoverableReadinessFailure(error))throw error;bridgeReplay=true;bridgeNext=performance.now()+Math.min(1000,50*2**(bridgeAttempts-1));}
            };
            for(;;){
                try{current=await this.read();}catch(error){if(!recoverableReadinessFailure(error))throw error;await this.delay();continue;}
                phase=this.phase(current,true)!;
                if(this.prearmed) {
                    const bridge=this.phaseOwner.observeBridge(current.webrtc_public_video_bridge);
                    if(bridge.feeding&&!bridgeMarker){
                        const feeding=bridge.feeding;
                        bridgeMarker=await this.prearmed.bridgeReady(feeding,async()=>{
                            let status:JsonObject;
                            try{status=await this.read();}catch(error){if(recoverableReadinessFailure(error))return undefined;throw error;}
                            const pending=this.phase(status,true)!;
                            const same=this.phaseOwner.observeBridge(status.webrtc_public_video_bridge);
                            if(pending.phase!=="negotiated-never-fed"||!same.feeding||bridgeCanonical(same.feeding)!==bridgeCanonical(feeding))throw invalid();
                            return parseWebRtcPreCutKeyframes(status.receiver_video_pre_cut_keyframes);
                        });
                        await sendBridge();
                    }
                    if(bridgeReplay&&bridgeAttempts<8&&performance.now()>=bridgeNext&&current.state==="preparing"&&phase.phase==="negotiated-never-fed")await sendBridge(current);
                    if(phase.phase==="cut-bound-post-f") {
                        if(!bridgeMarker||!bridge.decoded_ready||bridgeCanonical(bridgeMarker)!==bridgeCanonical(bridge.decoded_ready))throw invalid();
                        const raw=this.original.reservedReceiverWitness(current,false);
                        if(raw!==undefined){this.provisional=parseWebRtcReceiverPostCut(raw,this.original.reservedContext().root,phase);break;}
                        await this.delay();continue;
                    }
                } else if(phase.phase==="cut-bound-post-f")break;
                if(phase.phase!=="negotiated-never-fed")throw invalid();
                if(replay&&attempts<8&&performance.now()>=next&&current.state==="preparing"&&current.native_replacement_prepared===true)
                    await sendPrivate(current);
                else await this.delay();
            }
            this.observation=this.prearmed?await this.prearmed.postCut(this.provisional!):await observeWebRtcPublicVideoReadiness(this.phaseOwner,this.peer!,this.roles!,this.budget,this.signal,this.trackTask);
            this.prearmed=undefined;
            this.check();const marker=this.observation.readiness;
            attempts=0;replay=false;next=0;let decodedReady=false;
            const sendPublic=async(original?:JsonObject)=>{
                this.check();this.observation!.assertCurrent();attempts++;
                try{await this.run(()=>this.original.reservedPublicReady(marker,this.options(),original));replay=false;}
                catch(error){if(!recoverableReadinessFailure(error))throw error;replay=true;next=performance.now()+Math.min(1000,50*2**(attempts-1));}
            };
            await sendPublic();
            for(;;){
                try{current=await this.read();}catch(error){if(!recoverableReadinessFailure(error))throw error;await this.delay();continue;}
                phase=this.phase(current,true)!;
                if(this.provisional&&phase.phase!=="activated") {
                    const raw=this.original.reservedReceiverWitness(current,false);
                    // Decoded-ready confirms the original cut via the signed
                    // phase. The initial coordinate may be omitted thereafter.
                    if(raw!==undefined&&bridgeCanonical(parseWebRtcReceiverPostCut(raw,this.original.reservedContext().root,phase))!==bridgeCanonical(this.provisional))throw invalid();
                }
                if(phase.phase==="activated"){
                    if(current.state!=="completed")throw invalid();
                    const receipt=this.original.reservedCompleted(current);
                    let releaseOwner:ReceiverVideoReleaseOwner|undefined;
                    if(this.provisional) {
                        const raw=this.original.reservedReceiverWitness(current,true);
                        if(raw===undefined){await this.delay();continue;}
                        const expected={...this.provisional,profile:"connect-receiver-video-release-v1"};
                        if(bridgeCanonical(raw)!==bridgeCanonical(expected))throw invalid();
                        const peer=new WeakRef(this.peer!),roles=new WeakRef(this.roles!),binding=this.roles!.binding("public");if(!binding)throw invalid();
                        const track=new WeakRef(binding.track),receiver=new WeakRef(binding.receiver),owner=this.phaseOwner,mid=phase.binding.mid;
                        releaseOwner=new ReceiverVideoReleaseOwner(receiverScope(this.original.reservedContext().root,phase),{carrier:"webrtc-rtp",physical_writer_incarnation:phase.binding.physical_writer_incarnation,primary_ssrc:phase.binding.primary_ssrc},()=>{
                            const p=peer.deref(),r=roles.deref()?.binding("public");return !!p&&owner.exactLocalPeer(p)&&!!r&&r.track===track.deref()&&r.receiver===receiver.deref()&&r.mid===mid&&r.track.readyState==="live"&&!['closed','failed'].includes(p.connectionState);
                        });
                        releaseOwner.observe(raw,{operationRecordId:phase.operation_record_id,operationId:phase.operation_id,targetConnectionId:phase.target_connection_id,releaseAuthorized:true});
                    }
                    this.check();const binding=this.roles!.binding("public");if(!binding)throw invalid();
                    const picture=this.observation.take(phase,binding.track);if(!picture)throw invalid();
                    this.completed=new CompletedPublicVideo(picture,binding.track,releaseOwner);adopted=true;this.observation=undefined;
                    this.peer=undefined;this.roles=undefined;return receipt;
                }
                if(current.state==="completed")throw invalid();
                if(phase.phase==="decoded-ready"){decodedReady=true;replay=false;}
                else if(phase.phase!=="cut-bound-post-f")throw invalid();
                if(!decodedReady&&replay&&attempts<8&&performance.now()>=next&&current.state==="preparing"&&current.native_replacement_prepared===true)
                    await sendPublic(current);
                else await this.delay();
            }
        }catch(error){terminal=error;throw error;}finally{
            if(!adopted){
                const observation=this.observation??this.prearmed;this.observation=undefined;this.prearmed=undefined;this.phaseOwner.close();
                if(observation)try{await this.trackTask(observation.release(),true);}catch(error){if(terminal===undefined)throw error;}
                this.peer=undefined;this.roles=undefined;
            }
        }
    }
    /** No original operation budget check: this lease was transferred while
     * actual Completed+Activated and that deadline were valid. */
    assertActivated():void {
        if(this.stopped||!this.completed)throw new ConnectError("unsupported-media-profile","Reserved B has no original activated decoded lease");
        this.original.reservedContext();
    }
    borrow(track:MediaStreamTrack):WebRtcPublicVideoLoan {
        this.assertActivated();return this.completed!.borrow(track);
    }
    close():Promise<void> {
        if(this.stopped)return Promise.resolve();this.stopped=true;this.cancel.abort();this.phaseOwner.close();
        const pending=this.observation??this.prearmed,completed=this.completed;this.observation=undefined;this.prearmed=undefined;this.completed=undefined;
        this.peer=undefined;this.roles=undefined;
        return Promise.all([pending?.release(),completed?.release()]).then(()=>{});
    }
}
