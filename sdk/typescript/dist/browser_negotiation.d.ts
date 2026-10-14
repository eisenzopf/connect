import type { BrowserDeadline } from "./browser_deadline.js";
import type { BrowserOfferSelection } from "./browser_binding.js";
export interface BrowserSignal {
    type?: string;
    sdp?: string;
    connection_id?: string;
    request_id?: string;
}
/** One correlated offer/answer exchange; the caller owns cancellation and peer/socket release. */
export declare function negotiateBrowser(peer: RTCPeerConnection, socket: WebSocket, localStream: MediaStream, budget: BrowserDeadline, onConnection: (id: string) => void, selection?: BrowserOfferSelection): Promise<void>;
