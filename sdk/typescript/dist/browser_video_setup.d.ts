export { stopVideoWorker } from "./browser_video_keyframes.js";
/** Bound the original video encoding before an offer can activate media. */
export declare function addBoundedVideoSender(peer: RTCPeerConnection, track: MediaStreamTrack, stream: MediaStream): void;
/** Constrain application-supplied capture as well as getUserMedia capture. */
export declare function constrainVideoCapture(stream: MediaStream, timeoutMs: number): Promise<void>;
/** Owned encoded transform; its keyframe feedback is separate from capture. */
export declare function startVideoTransform(peer: RTCPeerConnection, selected?: RTCRtpTransceiver): Worker;
/** Configure negotiated send limits without allowing resolution adaptation. */
export declare function configureVideoSender(peer: RTCPeerConnection): Promise<void>;
