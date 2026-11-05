import { BrowserDeadline } from "./browser_deadline.js";
import { ConnectError } from "./errors.js";
import type { WebRtcDecodedReadiness } from "./browser_webrtc_replacement.js";

/** Readiness comes from this new RTC receiver's decoded PCM and video output.
 * ICE, DTLS, RTP counters, and an SDP answer cannot satisfy it on their own.
 * The private outputs remain muted; an application owns audible rendering. */
export async function observeWebRtcDecodedReadiness(
    peer: RTCPeerConnection,
    audio: { id: string; track: MediaStreamTrack },
    video: { id: string; track: MediaStreamTrack } | undefined,
    budget: BrowserDeadline,
    signal: AbortSignal,
    trackTask: <T>(work: Promise<T>, cleanup?: boolean) => Promise<T>,
): Promise<WebRtcDecodedReadiness> {
    const unsupported = () =>
        new ConnectError(
            "unsupported-media-profile",
            "WebRTC decoded replacement observation is unavailable",
        );
    if (
        typeof AudioContext !== "function" ||
        typeof AudioWorkletNode !== "function" ||
        audio.track.kind !== "audio" ||
        (video && video.track.kind !== "video")
    )
        throw unsupported();
    let context: AudioContext | undefined,
        source: MediaStreamAudioSourceNode | undefined,
        worklet: AudioWorkletNode | undefined,
        gain: GainNode | undefined,
        playout: HTMLAudioElement | undefined,
        picture: HTMLVideoElement | undefined,
        frameCallback: number | undefined,
        stopped = false,
        sampleCount = 0,
        frameCount = 0,
        presented = 0,
        failed: Error | undefined;
    const run = <T>(work: () => Promise<T>) =>
        budget.run(() => trackTask(work()), signal);
    const check = () => {
        if (failed) throw failed;
        if (
            signal.aborted ||
            stopped ||
            audio.track.readyState !== "live" ||
            (video && video.track.readyState !== "live") ||
            ["closed", "failed"].includes(peer.connectionState)
        )
            throw new ConnectError(
                "closed",
                "Replacement decoder owner retired",
            );
    };
    const clean = async () => {
        if (stopped) return;
        stopped = true;
        if (picture && frameCallback !== undefined)
            picture.cancelVideoFrameCallback(frameCallback);
        if (picture) {
            picture.pause();
            picture.srcObject = null;
        }
        if (playout) {
            playout.pause();
            playout.srcObject = null;
        }
        if (worklet) {
            worklet.port.onmessage = null;
            worklet.port.postMessage({ type: "stop" });
            worklet.port.close();
            worklet.disconnect();
        }
        source?.disconnect();
        gain?.disconnect();
        if (context) await trackTask(context.close(), true);
    };
    try {
        check();
        context = new AudioContext({
            sampleRate: 48000,
            latencyHint: "interactive",
        });
        if (context.sampleRate !== 48000) throw unsupported();
        await run(() =>
            context!.audioWorklet.addModule(
                new URL("./browser_audio_worklet.js", import.meta.url),
            ),
        );
        check();
        worklet = new AudioWorkletNode(context, "connect-pcm-48k", {
            channelCount: 1,
            channelCountMode: "explicit",
            numberOfInputs: 1,
            numberOfOutputs: 1,
            outputChannelCount: [1],
        });
        worklet.port.onmessage = (event: MessageEvent) => {
            if (stopped) return;
            const data = event.data;
            try {
                if (
                    data?.epoch !== 0 ||
                    !(data.samples instanceof Float32Array) ||
                    data.samples.length !== 960 ||
                    !Number.isSafeInteger(data.timestampUs) ||
                    data.samples.some(
                        (sample: number) => !Number.isFinite(sample),
                    )
                )
                    throw new ConnectError(
                        "media",
                        "Invalid decoded replacement PCM observation",
                    );
                sampleCount += data.samples.length;
            } catch (error) {
                failed = error instanceof Error ? error : unsupported();
            } finally {
                if (!stopped)
                    worklet?.port.postMessage({
                        type: "credit",
                        epoch: data?.epoch,
                    });
            }
        };
        source = context.createMediaStreamSource(
            new MediaStream([audio.track]),
        );
        gain = context.createGain();
        gain.gain.value = 0;
        source.connect(worklet);
        worklet.connect(gain);
        gain.connect(context.destination);
        playout = document.createElement("audio");
        playout.muted = true;
        playout.srcObject = new MediaStream([audio.track]);
        await run(() => context!.resume());
        await run(() => playout!.play());
        if (video) {
            picture = document.createElement("video");
            if (typeof picture.requestVideoFrameCallback !== "function")
                throw unsupported();
            picture.muted = true;
            picture.playsInline = true;
            picture.srcObject = new MediaStream([video.track]);
            const decoded: VideoFrameRequestCallback = (_now, metadata) => {
                if (stopped) return;
                if (
                    metadata.width > 0 &&
                    metadata.height > 0 &&
                    Number.isFinite(metadata.mediaTime) &&
                    Number.isSafeInteger(metadata.presentedFrames) &&
                    metadata.presentedFrames > presented
                ) {
                    frameCount++;
                    presented = metadata.presentedFrames;
                }
                frameCallback = picture!.requestVideoFrameCallback(decoded);
            };
            frameCallback = picture.requestVideoFrameCallback(decoded);
            await run(() => picture!.play());
        }
        for (;;) {
            check();
            const report = await run(() => peer.getStats());
            check();
            let decodedAudio = 0,
                decodedVideo = 0,
                decodedKeyframes: number | null = null;
            report.forEach((value) => {
                if (value.type !== "inbound-rtp" || value.isRemote === true)
                    return;
                const kind = value.kind ?? value.mediaType;
                if (
                    kind === "audio" &&
                    (!value.trackIdentifier ||
                        value.trackIdentifier === audio.track.id) &&
                    Number.isSafeInteger(value.totalSamplesReceived) &&
                    Number.isSafeInteger(value.concealedSamples)
                )
                    decodedAudio += Math.max(
                        0,
                        value.totalSamplesReceived - value.concealedSamples,
                    );
                if (
                    video &&
                    kind === "video" &&
                    (!value.trackIdentifier ||
                        value.trackIdentifier === video.track.id)
                ) {
                    if (Number.isSafeInteger(value.framesDecoded))
                        decodedVideo += value.framesDecoded;
                    if (Number.isSafeInteger(value.keyFramesDecoded))
                        decodedKeyframes =
                            (decodedKeyframes ?? 0) + value.keyFramesDecoded;
                }
            });
            if (
                sampleCount >= 960 &&
                decodedAudio >= 960 &&
                (!video ||
                    (frameCount > 0 &&
                        (decodedKeyframes === null || decodedKeyframes > 0)))
            ) {
                return Object.freeze({
                    audio: Object.freeze({
                        stream_id: audio.id,
                        decoded_samples: Math.min(sampleCount, decodedAudio),
                    }),
                    video: video
                        ? Object.freeze({
                              stream_id: video.id,
                              decoded_frames:
                                  decodedVideo > 0
                                      ? Math.min(frameCount, decodedVideo)
                                      : frameCount,
                              decoded_keyframes: decodedKeyframes,
                              decoder_startup:
                                  "fresh-peer-first-decoded-frame" as const,
                          })
                        : null,
                });
            }
            await run(
                () =>
                    new Promise<void>((resolve) =>
                        setTimeout(resolve, Math.min(20, budget.remaining())),
                    ),
            );
        }
    } finally {
        // Synchronous detachment stops playback now. The owner joins any late
        // browser cleanup without extending the caller's original deadline.
        void trackTask(clean(), true).catch(() => {});
    }
}
