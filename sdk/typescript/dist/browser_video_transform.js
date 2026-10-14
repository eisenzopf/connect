const workerScope = globalThis;
const observations = {
    observedFrames: 0,
    observedKeyframes: 0,
    keyframeRequests: 0,
    keyframeCompletions: 0,
    keyframeFailures: 0,
    keyframeRequestPending: false,
};
let pendingSince = 0;
const increment = (key) => {
    observations[key] = Math.min(Number.MAX_SAFE_INTEGER, observations[key] + 1);
};
workerScope.onmessage = ({ data, ports }) => {
    if (data?.op !== "video-diagnostics" || ports.length !== 1) {
        for (const port of ports)
            port.close();
        return;
    }
    try {
        ports[0].postMessage({
            ...observations,
            pendingRequestSeconds: observations.keyframeRequestPending
                ? Math.max(0, performance.now() - pendingSince) / 1000
                : 0,
        });
    }
    catch {
        // A released diagnostics port must not interrupt the encoded stream.
    }
    finally {
        ports[0].close();
    }
};
workerScope.onrtctransform = ({ transformer }) => {
    const adapterPort = transformer.options?.keyframePort;
    let requestId = 0;
    let pendingCompletion;
    let pendingFailure;
    let requestTimer;
    if (adapterPort)
        adapterPort.onmessage = ({ data }) => {
            if (data?.id === requestId && data.state === "failed") {
                const name = [
                    "TypeError",
                    "InvalidStateError",
                    "OperationError",
                    "NotSupportedError",
                    "AbortError",
                    "TimeoutError",
                ].includes(data.errorName)
                    ? data.errorName
                    : "Error";
                pendingFailure?.(new DOMException("Sender keyframe request failed", name));
            }
            // "applied" is only an API acceptance; observed keyframe completes it.
        };
    const requestKeyframe = () => {
        if (typeof transformer.generateKeyFrame === "function")
            return transformer.generateKeyFrame();
        if (!adapterPort)
            return Promise.reject(new TypeError("No sender keyframe adapter"));
        return new Promise((resolve, reject) => {
            const finish = (failure) => {
                clearTimeout(requestTimer);
                requestTimer = undefined;
                pendingCompletion = undefined;
                pendingFailure = undefined;
                if (failure)
                    reject(failure);
                else
                    resolve();
            };
            pendingCompletion = () => finish();
            pendingFailure = finish;
            requestTimer = setTimeout(() => finish(new DOMException("No observed generated keyframe", "TimeoutError")), 2000);
            try {
                adapterPort.postMessage({ op: "keyframe", id: ++requestId });
            }
            catch (error) {
                finish(error);
            }
        });
    };
    let lastKey = 0;
    let lastRequest = 0;
    let pending = false;
    const stream = new TransformStream({
        transform(frame, controller) {
            const now = performance.now();
            increment("observedFrames");
            if (frame.type === "key") {
                lastKey = now;
                increment("observedKeyframes");
                pendingCompletion?.();
            }
            if (!pending && now - lastKey >= 900 && now - lastRequest >= 250) {
                pending = true;
                lastRequest = now;
                increment("keyframeRequests");
                observations.keyframeRequestPending = true;
                pendingSince = now;
                void Promise.resolve()
                    .then(requestKeyframe)
                    .then(() => increment("keyframeCompletions"))
                    .catch((error) => {
                    increment("keyframeFailures");
                    // Fixed operation/context only; no media or caller IDs.
                    const name = typeof error === "object" &&
                        error !== null &&
                        "name" in error &&
                        typeof error.name === "string"
                        ? error.name
                        : "Error";
                    workerScope.postMessage({
                        state: "keyframe-unavailable",
                        errorName: [
                            "TypeError",
                            "InvalidStateError",
                            "OperationError",
                            "NotSupportedError",
                            "AbortError",
                            "TimeoutError",
                        ].includes(name)
                            ? name
                            : "Error",
                    });
                })
                    .finally(() => {
                    pending = false;
                    observations.keyframeRequestPending = false;
                });
            }
            controller.enqueue(frame);
        },
    });
    void transformer.readable
        .pipeThrough(stream)
        .pipeTo(transformer.writable)
        .catch(() => {
        pendingFailure?.(new Error("Encoder ended"));
        workerScope.postMessage({ state: "encoder-ended" });
    })
        .finally(() => {
        clearTimeout(requestTimer);
        adapterPort?.close();
    });
};
export {};
