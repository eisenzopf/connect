import type { BrowserDeadline } from "./browser_deadline.js";
import type { BrowserOfferSelection } from "./browser_binding.js";
import { validateVideoAnswer, waitForIce } from "./browser_capture.js";
import { ConnectError } from "./errors.js";

export interface BrowserSignal {
    type?: string;
    sdp?: string;
    connection_id?: string;
    request_id?: string;
}

/** One correlated offer/answer exchange; the caller owns cancellation and peer/socket release. */
export async function negotiateBrowser(
    peer: RTCPeerConnection,
    socket: WebSocket,
    localStream: MediaStream,
    budget: BrowserDeadline,
    onConnection: (id: string) => void,
    selection?: BrowserOfferSelection,
): Promise<void> {
    const requestId = crypto.randomUUID();
    const offer = await peer.createOffer();
    await peer.setLocalDescription(offer);
    await waitForIce(peer, budget.remaining());
    const local = peer.localDescription;
    if (!local?.sdp || local.sdp.length > 1024 * 1024)
        throw new ConnectError("protocol", "Invalid local WebRTC offer");
    await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(
            () =>
                reject(
                    new ConnectError("timeout", "WebRTC signaling timed out"),
                ),
            budget.remaining(),
        );
        let answerApplied = false;
        let readyReceived = false;
        const finishIfReady = () => {
            if (!answerApplied || !readyReceived) return;
            clearTimeout(timer);
            resolve();
        };
        const fail = () => {
            clearTimeout(timer);
            reject(
                new ConnectError("webrtc-failed", "WebRTC signaling closed"),
            );
        };
        socket.onerror = fail;
        socket.onclose = fail;
        const sendOffer = () => {
            try {
                socket.send(
                    JSON.stringify({
                        type: "offer-ready",
                        sdp: local.sdp,
                        request_id: requestId,
                        ...selection,
                    }),
                );
            } catch {
                fail();
            }
        };
        socket.onopen = sendOffer;
        socket.onmessage = async (event) => {
            try {
                if (
                    typeof event.data !== "string" ||
                    event.data.length > 1024 * 1024
                )
                    throw new Error();
                const signal = JSON.parse(event.data) as BrowserSignal;
                if (
                    signal.request_id !== undefined &&
                    typeof signal.request_id !== "string"
                )
                    throw new ConnectError(
                        "protocol",
                        "Invalid signaling correlation",
                    );
                if (signal.request_id && signal.request_id !== requestId)
                    return;
                if (signal.type === "answer") {
                    if (
                        typeof signal.sdp !== "string" ||
                        !signal.sdp ||
                        signal.sdp.length > 1024 * 1024 ||
                        typeof signal.connection_id !== "string" ||
                        !signal.connection_id ||
                        signal.connection_id.length > 128 ||
                        /[\s\u0000-\u001f\u007f-\u009f]/u.test(
                            signal.connection_id,
                        )
                    )
                        throw new ConnectError(
                            "protocol",
                            "Invalid signaling answer",
                        );
                    if (
                        localStream
                            .getTracks()
                            .some((track) => track.kind === "video")
                    )
                        validateVideoAnswer(signal.sdp);
                    onConnection(signal.connection_id);
                    await peer.setRemoteDescription({
                        type: "answer",
                        sdp: signal.sdp,
                    });
                    answerApplied = true;
                    finishIfReady();
                } else if (signal.type === "ready") {
                    readyReceived = true;
                    finishIfReady();
                } else if (signal.type === "rejected") {
                    throw new ConnectError(
                        "forbidden",
                        "Voice invitation was rejected",
                    );
                }
            } catch (error) {
                clearTimeout(timer);
                reject(error);
            }
        };
        // ICE gathering can outlast the WebSocket handshake. Install the
        // answer handler before sending, including when `open` already ran.
        if (socket.readyState === WebSocket.OPEN) sendOffer();
        else if (socket.readyState !== WebSocket.CONNECTING) fail();
    });
}
