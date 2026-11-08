import { ConnectError } from "./errors.js";
/** Readiness comes from this new RTC receiver's decoded PCM and video output.
 * ICE, DTLS, RTP counters, and an SDP answer cannot satisfy it on their own.
 * The private outputs remain muted; an application owns audible rendering. */
export async function observeWebRtcDecodedReadiness(peer, audio, video, budget, signal, trackTask, retainMutedDecoderSinks) {
    const unsupported = () => new ConnectError("unsupported-media-profile", "WebRTC decoded replacement observation is unavailable");
    if (typeof AudioContext !== "function" ||
        typeof AudioWorkletNode !== "function" ||
        audio.track.kind !== "audio" ||
        (video && video.track.kind !== "video"))
        throw unsupported();
    let context, source, worklet, gain, playout, picture, frameCallback, stopped = false, decoderSinksStopped = false, decoderSinksRetained = false, audioAdopted = false, sampleCount = 0, frameCount = 0, presented = 0, failed;
    const run = (work) => budget.run(() => trackTask(work()), signal);
    const check = () => {
        if (failed)
            throw failed;
        if (signal.aborted ||
            stopped ||
            audio.track.readyState !== "live" ||
            (video && video.track.readyState !== "live") ||
            ["closed", "failed"].includes(peer.connectionState))
            throw new ConnectError("closed", "Replacement decoder owner retired");
    };
    // This lease transfers only this exact peer's muted decoded graph once.
    // The renderer owns adopted outputs; release owns only outputs left here.
    const releaseDecoderSinks = async () => {
        if (decoderSinksStopped)
            return;
        decoderSinksStopped = true;
        signal.removeEventListener("abort", cancelDecoderSinks);
        const originalPicture = picture;
        const elements = [picture, playout];
        picture = undefined;
        playout = undefined;
        let failure;
        if (originalPicture && frameCallback !== undefined) {
            try {
                originalPicture.cancelVideoFrameCallback(frameCallback);
            }
            catch (error) {
                failure ??= { error };
            }
        }
        frameCallback = undefined;
        for (const element of elements) {
            if (!element)
                continue;
            try {
                element.pause();
            }
            catch (error) {
                failure ??= { error };
            }
            try {
                element.srcObject = null;
            }
            catch (error) {
                failure ??= { error };
            }
        }
        source?.disconnect();
        gain?.disconnect();
        source = undefined;
        gain = undefined;
        const originalContext = context;
        context = undefined;
        if (originalContext) {
            try {
                await trackTask(originalContext.close(), true);
            }
            catch (error) {
                failure ??= { error };
            }
        }
        if (failure)
            throw failure.error;
    };
    const cancelDecoderSinks = () => {
        void trackTask(releaseDecoderSinks(), true).catch(() => { });
    };
    const cleanObservation = async () => {
        if (stopped)
            return;
        stopped = true;
        if (picture && frameCallback !== undefined)
            picture.cancelVideoFrameCallback(frameCallback);
        frameCallback = undefined;
        if (worklet) {
            worklet.port.onmessage = null;
            worklet.port.postMessage({ type: "stop" });
            worklet.port.close();
            worklet.disconnect();
        }
        if (worklet)
            source?.disconnect(worklet);
        worklet = undefined;
    };
    const decoder = Object.freeze({
        takeAudio(track) {
            if (audioAdopted ||
                decoderSinksStopped ||
                signal.aborted ||
                track !== audio.track ||
                track.readyState !== "live" ||
                ["closed", "failed"].includes(peer.connectionState) ||
                !context ||
                !source ||
                !gain ||
                !playout)
                return undefined;
            // No await can interleave cancellation between transfer and the
            // renderer's local assignment. The old signal no longer owns it.
            signal.removeEventListener("abort", cancelDecoderSinks);
            const output = Object.freeze({
                context,
                source,
                mutedGain: gain,
                playout,
            });
            audioAdopted = true;
            context = undefined;
            source = undefined;
            gain = undefined;
            playout = undefined;
            return output;
        },
        release: releaseDecoderSinks,
    });
    try {
        check();
        context = new AudioContext({
            sampleRate: 48000,
            latencyHint: "interactive",
        });
        if (context.sampleRate !== 48000)
            throw unsupported();
        await run(() => context.audioWorklet.addModule(new URL("./browser_audio_worklet.js", import.meta.url)));
        check();
        worklet = new AudioWorkletNode(context, "connect-pcm-48k", {
            channelCount: 1,
            channelCountMode: "explicit",
            numberOfInputs: 1,
            numberOfOutputs: 1,
            outputChannelCount: [1],
        });
        worklet.port.onmessage = (event) => {
            if (stopped)
                return;
            const data = event.data;
            try {
                if (data?.epoch !== 0 ||
                    !(data.samples instanceof Float32Array) ||
                    data.samples.length !== 960 ||
                    !Number.isSafeInteger(data.timestampUs) ||
                    data.samples.some((sample) => !Number.isFinite(sample)))
                    throw new ConnectError("media", "Invalid decoded replacement PCM observation");
                sampleCount += data.samples.length;
            }
            catch (error) {
                failed = error instanceof Error ? error : unsupported();
            }
            finally {
                if (!stopped)
                    worklet?.port.postMessage({
                        type: "credit",
                        epoch: data?.epoch,
                    });
            }
        };
        source = context.createMediaStreamSource(new MediaStream([audio.track]));
        gain = context.createGain();
        gain.gain.value = 0;
        source.connect(worklet);
        // Keep this exact source graph advancing while observation is removed
        // and before the renderer transfers it to its own public output gain.
        source.connect(gain);
        worklet.connect(gain);
        gain.connect(context.destination);
        playout = document.createElement("audio");
        playout.muted = true;
        playout.srcObject = new MediaStream([audio.track]);
        await run(() => context.resume());
        await run(() => playout.play());
        if (video) {
            picture = document.createElement("video");
            if (typeof picture.requestVideoFrameCallback !== "function")
                throw unsupported();
            picture.muted = true;
            picture.playsInline = true;
            picture.srcObject = new MediaStream([video.track]);
            const decoded = (_now, metadata) => {
                if (stopped)
                    return;
                if (metadata.width > 0 &&
                    metadata.height > 0 &&
                    Number.isFinite(metadata.mediaTime) &&
                    Number.isSafeInteger(metadata.presentedFrames) &&
                    metadata.presentedFrames > presented) {
                    frameCount++;
                    presented = metadata.presentedFrames;
                }
                frameCallback = picture.requestVideoFrameCallback(decoded);
            };
            frameCallback = picture.requestVideoFrameCallback(decoded);
            await run(() => picture.play());
        }
        for (;;) {
            check();
            const report = await run(() => peer.getStats());
            check();
            let decodedAudio = 0, decodedVideo = 0, decodedKeyframes = null;
            report.forEach((value) => {
                if (value.type !== "inbound-rtp" || value.isRemote === true)
                    return;
                const kind = value.kind ?? value.mediaType;
                if (kind === "audio" &&
                    (!value.trackIdentifier ||
                        value.trackIdentifier === audio.track.id) &&
                    Number.isSafeInteger(value.totalSamplesReceived) &&
                    Number.isSafeInteger(value.concealedSamples))
                    decodedAudio += Math.max(0, value.totalSamplesReceived - value.concealedSamples);
                if (video &&
                    kind === "video" &&
                    (!value.trackIdentifier ||
                        value.trackIdentifier === video.track.id)) {
                    if (Number.isSafeInteger(value.framesDecoded))
                        decodedVideo += value.framesDecoded;
                    if (Number.isSafeInteger(value.keyFramesDecoded))
                        decodedKeyframes =
                            (decodedKeyframes ?? 0) + value.keyFramesDecoded;
                }
            });
            if (sampleCount >= 960 &&
                decodedAudio >= 960 &&
                (!video ||
                    (frameCount > 0 &&
                        (decodedKeyframes === null ||
                            (decodedKeyframes > 0 &&
                                decodedKeyframes <=
                                    (decodedVideo > 0
                                        ? decodedVideo
                                        : frameCount)))))) {
                if (retainMutedDecoderSinks) {
                    check();
                    signal.addEventListener("abort", cancelDecoderSinks, {
                        once: true,
                    });
                    retainMutedDecoderSinks(decoder);
                    check();
                    decoderSinksRetained = true;
                }
                return Object.freeze({
                    audio: Object.freeze({
                        stream_id: audio.id,
                        decoded_samples: Math.min(sampleCount, decodedAudio),
                    }),
                    video: video
                        ? Object.freeze({
                            stream_id: video.id,
                            decoded_frames: decodedVideo > 0 ? decodedVideo : frameCount,
                            decoded_keyframes: decodedKeyframes,
                            decoder_startup: "fresh-peer-first-decoded-frame",
                        })
                        : null,
                });
            }
            await run(() => new Promise((resolve) => setTimeout(resolve, Math.min(20, budget.remaining()))));
        }
    }
    finally {
        // Observation detaches immediately. The native owner retains the muted
        // decoded graph through public startup and joins late cleanup without
        // extending the caller's original deadline.
        void trackTask(cleanObservation(), true).catch(() => { });
        if (!decoderSinksRetained)
            cancelDecoderSinks();
    }
}
