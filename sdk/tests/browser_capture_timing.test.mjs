import assert from "node:assert/strict";
import { test } from "node:test";
import { BrowserMediaCapture } from "../typescript/dist/browser_media_capture.js";
async function environment(run) {
    const names = [
            "navigator",
            "document",
            "MediaStream",
            "HTMLVideoElement",
            "VideoFrame",
            "performance",
        ],
        old = names.map((n) => [
            n,
            Object.getOwnPropertyDescriptor(globalThis, n),
        ]);
    let now = 1000,
        callback,
        cancelled = 0,
        permission = 0;
    const clones = [],
        encoded = [],
        states = [],
        observers = new Set();
    class Track {
        kind = "video";
        readyState = "live";
        enabled = true;
        stops = 0;
        getSettings() {
            return { width: 640, height: 360, frameRate: 30 };
        }
        async applyConstraints() {}
        addEventListener() {}
        removeEventListener() {}
        stop() {
            this.stops++;
            this.readyState = "ended";
        }
        clone() {
            const t = new Track();
            clones.push(t);
            return t;
        }
    }
    class Stream {
        constructor(tracks) {
            this.tracks = tracks;
        }
        getTracks() {
            return this.tracks;
        }
        getVideoTracks() {
            return this.tracks.filter((t) => t.kind === "video");
        }
        getAudioTracks() {
            return [];
        }
    }
    class Video {
        muted = false;
        playsInline = false;
        async play() {}
        pause() {}
        requestVideoFrameCallback(fn) {
            callback = fn;
            return 1;
        }
        cancelVideoFrameCallback() {
            cancelled++;
            callback = undefined;
        }
    }
    const values = [
        {
            mediaDevices: {
                getUserMedia() {
                    permission++;
                    throw Error("unexpected permission");
                },
            },
        },
        {
            visibilityState: "visible",
            addEventListener() {},
            removeEventListener() {},
            createElement: (n) =>
                n === "video"
                    ? new Video()
                    : { getContext: () => ({ drawImage() {} }) },
        },
        Stream,
        Video,
        class {
            constructor(_canvas, init) {
                this.timestamp = init.timestamp;
            }
            close() {
                this.closed = true;
            }
        },
        { now: () => now },
    ];
    for (let i = 0; i < names.length; i++)
        Object.defineProperty(globalThis, names[i], {
            configurable: true,
            value: values[i],
        });
    const original = new Track(),
        connection = {
            id: "connection",
            streams: [
                {
                    streamId: "video",
                    kind: "video",
                    codec: "vp8",
                    direction: "sendrecv",
                },
            ],
            ownCleanup(work){void work.catch(()=>{});},onClose(fn) {
                observers.add(fn);
                return () => observers.delete(fn);
            },
            encodeVideo(id, f) {
                assert.equal(id, "video");
                encoded.push(f.timestamp);
                f.close();
                return true;
            },
        };
    let capture;
    try {
        capture = await BrowserMediaCapture.open(connection, {
            videoStreamId: "video",
            stream: new Stream([original]),
            state: (s, e) => states.push({ s, error: e?.code }),
        });
        await run({
            capture,
            encoded,
            states,
            clones,
            observers,
            original,
            fire(metadata) {
                assert.equal(typeof callback, "function");
                callback(now, metadata);
            },
            time(value) {
                now = value;
            },
            get cancelled() {
                return cancelled;
            },
            get permission() {
                return permission;
            },
        });
    } finally {
        capture?.close();
        for (const [n, d] of old)
            if (d) Object.defineProperty(globalThis, n, d);
            else delete globalThis[n];
    }
}
test("video capture epoch follows frame submission metadata despite delayed first and later callbacks", () =>
    environment(async (e) => {
        e.fire({ mediaTime: 0, presentationTime: 100 });
        e.time(2000);
        e.fire({ mediaTime: 0.04, presentationTime: 140 });
        assert.deepEqual(e.encoded, [100000, 140000]);
        assert.equal(e.permission, 0);
        assert.equal(e.original.stops, 0);
    }));
test("video capture prefers direct capture time including zero over presentation time", () =>
    environment(async (e) => {
        e.fire({ mediaTime: 0, presentationTime: 100, captureTime: 0 });
        e.fire({
            mediaTime: 0.04,
            presentationTime: 140,
            captureTime: 123.456,
        });
        assert.deepEqual(e.encoded, [0, 123456]);
    }));
test("unavailable or malformed frame timing closes capture and its owned track without encoding", async () => {
    for (const metadata of [
        { mediaTime: 0, presentationTime: 100, captureTime: NaN },
        { mediaTime: 0, presentationTime: 100, captureTime: -0.0001 },
        { mediaTime: 0, presentationTime: 100, captureTime: Infinity },
        {
            mediaTime: 0,
            presentationTime: 100,
            captureTime: Number.MAX_SAFE_INTEGER,
        },
        { mediaTime: 0 },
        { mediaTime: NaN, presentationTime: 100 },
        { mediaTime: -1, presentationTime: 100 },
        { mediaTime: 0, presentationTime: -1 },
        { mediaTime: 0, presentationTime: Infinity },
        { mediaTime: Number.MAX_SAFE_INTEGER, presentationTime: 100 },
        { mediaTime: 0, presentationTime: Number.MAX_SAFE_INTEGER },
    ])
        await environment(async (e) => {
            e.fire(metadata);
            assert.equal(e.capture.closed, true);
            assert.equal(e.encoded.length, 0);
            assert.equal(e.original.stops, 1);
            assert.equal(e.observers.size, 0);
            assert.ok(e.cancelled);
            assert.ok(e.states.some((s) => s.error === "media-capture"));
        });
});
test("paused video discards callbacks and resume preserves the frame epoch without rebasing to callback time", () =>
    environment(async (e) => {
        e.fire({ mediaTime: 0, presentationTime: 100 });
        e.capture.pause();
        e.fire({ mediaTime: 0.1, presentationTime: 200 });
        assert.deepEqual(e.encoded, [100000]);
        await e.capture.resume();
        e.time(3000);
        e.fire({ mediaTime: 2.4, presentationTime: 2500 });
        assert.deepEqual(e.encoded, [100000, 2500000]);
    }));
