/** Bounded sender-side keyframe adapter for encoded workers whose transformer
 * lacks generateKeyFrame. Implements the WebRTC setParameters keyFrame option:
 * https://w3c.github.io/webrtc-extensions/#rtcrtpsender-setparameters-keyframe
 * Promise resolution means API acceptance, never independently observed keys.
 */
interface KeyframeSender {
    getParameters(): RTCRtpSendParameters;
    setParameters(
        parameters: RTCRtpSendParameters,
        options: {
            encodingOptions: Array<{ keyFrame: boolean }>;
        },
    ): Promise<void>;
}
const owned = new WeakMap<Worker, () => void>();

export function keyframeAdapter(
    worker: Worker,
    sender: RTCRtpSender,
): MessagePort {
    const channel = new MessageChannel();
    let closed = false;
    let busy = false;
    let lastRequest = -Infinity;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const reply = (
        id: number,
        state: "applied" | "failed",
        errorName?: string,
    ) => {
        if (closed) return;
        try {
            channel.port1.postMessage({ id, state, errorName });
        } catch {
            /* released worker */
        }
    };
    channel.port1.onmessage = ({ data }) => {
        if (
            !data ||
            data.op !== "keyframe" ||
            !Number.isSafeInteger(data.id) ||
            data.id < 1
        )
            return;
        const id = data.id;
        if (busy || performance.now() - lastRequest < 250) {
            reply(id, "failed", "OperationError");
            return;
        }
        busy = true;
        lastRequest = performance.now();
        let finished = false;
        const finish = (state: "applied" | "failed", name?: string) => {
            if (finished) return;
            finished = true;
            clearTimeout(timer);
            timer = undefined;
            busy = false;
            reply(id, state, name);
        };
        timer = setTimeout(() => finish("failed", "TimeoutError"), 1000);
        void Promise.resolve()
            .then(() => {
                if (closed) return;
                const parameters = sender.getParameters();
                if (
                    !parameters.encodings?.length ||
                    !parameters.encodings.some((row) => row.active !== false)
                )
                    throw new DOMException(
                        "No active encoding",
                        "InvalidStateError",
                    );
                return (sender as unknown as KeyframeSender).setParameters(
                    parameters,
                    {
                        encodingOptions: parameters.encodings.map((row) => ({
                            keyFrame: row.active !== false,
                        })),
                    },
                );
            })
            .then(() => finish("applied"))
            .catch((error: unknown) => {
                const name = error instanceof Error ? error.name : "Error";
                finish(
                    "failed",
                    [
                        "TypeError",
                        "InvalidStateError",
                        "OperationError",
                        "NotSupportedError",
                        "AbortError",
                    ].includes(name)
                        ? name
                        : "Error",
                );
            });
    };
    owned.set(worker, () => {
        closed = true;
        clearTimeout(timer);
        channel.port1.onmessage = null;
        channel.port1.close();
        channel.port2.close();
    });
    return channel.port2;
}

export function stopVideoWorker(worker?: Worker): void {
    if (!worker) return;
    owned.get(worker)?.();
    owned.delete(worker);
    worker.terminate();
}
