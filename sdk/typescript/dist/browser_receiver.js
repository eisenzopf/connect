import { ConnectError } from "./errors.js";
/** A native playout preference, not a guarantee of measured latency. Keep
 * synchronized audio/video on the same target before signaling starts. */
export function configureReceiverBuffer(peer) {
    for (const { receiver } of peer.getTransceivers()) {
        try {
            if (!("jitterBufferTarget" in receiver))
                throw Error();
            receiver.jitterBufferTarget = 80;
            if (receiver.jitterBufferTarget !== 80)
                throw Error();
        }
        catch {
            throw new ConnectError("unsupported-media-profile", "Native receiver did not retain the 80 ms playout target");
        }
    }
}
