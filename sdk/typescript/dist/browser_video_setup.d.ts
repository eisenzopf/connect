export { stopVideoWorker } from "./browser_video_keyframes.js";
/** Constrain application-supplied capture as well as getUserMedia capture. */
export declare function constrainVideoCapture(stream: MediaStream, timeoutMs: number): Promise<void>;
/** Owned encoded transform; its keyframe feedback is separate from capture. */
export declare function startVideoTransform(peer: RTCPeerConnection): Worker;
/** Configure negotiated send limits without allowing resolution adaptation. */
export declare function configureVideoSender(peer: RTCPeerConnection): Promise<void>;
