import { ConnectError } from "./errors.js";
/** A graph source is borrowed; its context and unrelated edges remain caller-owned. */
export function captureAudioNode(input, audio) {
    const node = input.audioNode;
    if (node === undefined)
        return undefined;
    if (!audio ||
        input.audioDeviceId !== undefined ||
        input.stream?.getAudioTracks().length)
        throw new ConnectError("invalid-request", "Choose one audio source: device, supplied track or AudioNode");
    if (typeof AudioNode !== "function" ||
        !(node instanceof AudioNode) ||
        typeof AudioContext !== "function" ||
        !(node.context instanceof AudioContext) ||
        node.context.sampleRate !== 48000 ||
        node.context.state !== "running" ||
        node.numberOfOutputs < 1)
        throw new ConnectError("unsupported-media-profile", "AudioNode capture requires a running caller-owned 48 kHz real-time context");
    return node;
}
const captureModules = new WeakMap();
export function captureWorkletModule(context, signal) {
    const cancelled = () => new ConnectError("cancelled", "Capture module wait cancelled");
    if (signal.aborted)
        return Promise.reject(cancelled());
    let registration = captureModules.get(context);
    const start = registration === undefined;
    if (!registration) {
        registration = { ready: false, waiters: new Set() };
        captureModules.set(context, registration);
    }
    if (registration.ready)
        return Promise.resolve();
    const original = registration;
    const waiting = new Promise((resolve, reject) => {
        const abort = () => waiter.finish({ value: cancelled() });
        const waiter = {
            finish(error) {
                if (!original.waiters.delete(waiter))
                    return;
                signal.removeEventListener("abort", abort);
                if (error)
                    reject(error.value);
                else
                    resolve();
            },
        };
        original.waiters.add(waiter);
        signal.addEventListener("abort", abort, { once: true });
        if (signal.aborted)
            abort();
    });
    if (start) {
        const failed = (error) => {
            if (captureModules.get(context) === original)
                captureModules.delete(context);
            for (const waiter of [...original.waiters])
                waiter.finish({ value: error });
        };
        try {
            void context.audioWorklet
                .addModule(new URL("./browser_audio_worklet.js", import.meta.url))
                .then(() => {
                original.ready = true;
                for (const waiter of [...original.waiters])
                    waiter.finish();
            }, failed);
        }
        catch (error) {
            failed(error);
        }
    }
    return waiting;
}
