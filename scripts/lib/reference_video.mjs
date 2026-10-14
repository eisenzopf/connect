/** Real reference UI AV lifecycle through public SDK/API; generated capture only. */
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { ReferenceContinuity } from "./reference_continuity.mjs";

async function generatedCapture(page, frequency) {
    await page.addInitScript((frequency) => {
        globalThis.fixtureCaptureCount = 0;
        globalThis.fixturePeers = [];
        const RealPeer = RTCPeerConnection;
        globalThis.RTCPeerConnection = class extends RealPeer {
            constructor(...args) {
                super(...args);
                globalThis.fixturePeers.push(this);
            }
        };
        globalThis.fixtureOwnedTracks = [];
        navigator.mediaDevices.enumerateDevices = async () => [
            {
                kind: "audioinput",
                deviceId: "fixture-microphone",
                label: "Generated microphone",
            },
            {
                kind: "videoinput",
                deviceId: "fixture-camera",
                label: "Generated camera",
            },
        ];
        navigator.mediaDevices.getUserMedia = async (constraints) => {
            if (
                constraints.audio?.deviceId?.exact !== "fixture-microphone" ||
                constraints.video?.deviceId?.exact !== "fixture-camera"
            )
                throw Error(
                    "Selected fixture devices were not passed to capture",
                );
            if (!constraints.video)
                throw Error("fixture requires explicit audio/video");
            globalThis.fixtureCaptureCount++;
            const canvas = document.createElement("canvas");
            canvas.width = 640;
            canvas.height = 360;
            const paint = canvas.getContext("2d");
            let frame = 0;
            const draw = () => {
                paint.fillStyle = `hsl(${frame++ % 360} 80% 40%)`;
                paint.fillRect(0, 0, 640, 360);
                paint.fillStyle = "white";
                paint.fillRect((frame * 7) % 580, 90, 60, 100);
            };
            draw();
            const timer = setInterval(draw, 1000 / 30);
            const stream = canvas.captureStream(30);
            const audio = new AudioContext();
            await audio.resume();
            const tone = audio.createOscillator();
            tone.frequency.value = frequency;
            const output = audio.createMediaStreamDestination();
            tone.connect(output);
            tone.start();
            stream.addTrack(output.stream.getAudioTracks()[0]);
            globalThis.fixtureOwnedTracks.push(...stream.getTracks());
            let released = false;
            for (const track of stream.getTracks()) {
                const stop = track.stop.bind(track);
                track.stop = () => {
                    stop();
                    if (
                        !released &&
                        stream
                            .getTracks()
                            .every((item) => item.readyState === "ended")
                    ) {
                        released = true;
                        clearInterval(timer);
                        tone.stop();
                        void audio.close();
                    }
                };
            }
            return stream;
        };
    }, frequency);
}
async function click(page, id, expected) {
    await page.locator("#" + id).click();
    await page.waitForFunction(
        (state) =>
            document.querySelector("#state").textContent === state &&
            document.querySelector("#detail").textContent === "Complete.",
        expected,
        { timeout: 20000 },
    );
}
async function configuredPage(
    browser,
    baseURL,
    httpURL,
    config,
    counterpart,
    conversation,
    errors,
    frequency,
) {
    const page = await browser.newPage();
    page.on("pageerror", (error) => errors.push(error.name));
    await generatedCapture(page, frequency);
    await page.goto(baseURL + "/examples/conversation-client/");
    for (const [id, value] of Object.entries({
        http: httpURL,
        credential: config.credential,
        identity: config.identity_id,
        participant: counterpart,
        conversation,
    }))
        await page.locator("#" + id).fill(value);
    await click(page, "connect", "Connected");
    await click(page, "refresh-devices", "Devices listed");
    await page.locator("#audio-device").selectOption("fixture-microphone");
    await page.locator("#video-device").selectOption("fixture-camera");
    return page;
}
async function invite(initiator, recipient, accept) {
    await click(initiator, "invite-video", "Video invited");
    const session = await initiator.locator("#session").inputValue();
    assert(session.length > 0);
    await recipient.locator("#session").fill(session);
    await click(
        recipient,
        accept ? "accept" : "reject",
        accept ? "Session accepted" : "Session rejected",
    );
    return session;
}
async function decoded(page, expectedTone) {
    await page.waitForFunction(
        () =>
            document.querySelector("#remote-video").videoWidth === 640 &&
            document.querySelector("#remote-video").videoHeight === 360,
        undefined,
        { timeout: 20000 },
    );
    await click(page, "video-diagnostics", "Video inspected");
    const rows = JSON.parse(await page.locator("#video-proof").textContent());
    const received = rows.find((row) => row.direction === "inbound");
    assert(
        received &&
            received.frames > 0 &&
            received.codec?.toLowerCase() === "video/vp8",
    );
    const audio = await page.evaluate(async (expected) => {
        const track = document
            .querySelector("#remote-video")
            .srcObject.getAudioTracks()[0];
        if (!track || track.readyState !== "live")
            throw Error("No live remote audio track");
        const context = new AudioContext();
        try {
            await context.resume();
            const input = context.createMediaStreamSource(
                new MediaStream([track]),
            );
            const analyser = context.createAnalyser();
            analyser.fftSize = 2048;
            input.connect(analyser);
            const readings = [];
            for (let index = 0; index < 10; index++) {
                await new Promise((resolve) => setTimeout(resolve, 30));
                const samples = new Float32Array(2048);
                analyser.getFloatTimeDomainData(samples);
                const rms = Math.sqrt(
                    samples.reduce((sum, value) => sum + value * value, 0) /
                        samples.length,
                );
                let edges = 0;
                for (let i = 1; i < samples.length; i++)
                    if (samples[i - 1] <= 0 && samples[i] > 0) edges++;
                const hz = (edges * context.sampleRate) / samples.length;
                if (rms > 0.01 && Math.abs(hz - expected) < 35)
                    readings.push({ rms, hz });
            }
            if (readings.length < 5)
                throw Error(
                    "Remote generated audio was not independently identified",
                );
            return { matched_windows: readings.length, expected_hz: expected };
        } finally {
            await context.close();
        }
    }, expectedTone);
    return {
        audio,
        frames: received.frames,
        width: received.width,
        height: received.height,
        codec: received.codec,
    };
}
async function reset(page) {
    await page.waitForFunction(
        () =>
            document.querySelector("#join-video").disabled === false &&
            document.querySelector("#local-video").srcObject === null &&
            document.querySelector("#remote-video").srcObject === null,
        undefined,
        { timeout: 5000 },
    );
    assert.equal(
        await page.locator("#mute-camera").textContent(),
        "Mute camera",
    );
    assert.equal(
        await page.locator("#mute-microphone").textContent(),
        "Mute microphone",
    );
    assert.equal(
        await page.evaluate(() =>
            globalThis.fixtureOwnedTracks.every(
                (track) => track.readyState === "ended",
            ),
        ),
        true,
    );
}

export async function qualifyReferenceVideo({
    browser,
    baseURL,
    httpURL,
    config,
    peer,
    conversation,
    artifactId,
    client,
    peerClient,
    configureAndRestart,
    restartServer,
    scratch,
}) {
    const errors = [];
    await configureAndRestart();
    await client.send(
        "reference-av-context",
        conversation,
        "Written AV context: the chosen color is violet.",
    );
    const continuity = new ReferenceContinuity(
        client,
        peerClient,
        conversation,
        config.identity_id,
        peer.identity_id,
    );
    await continuity.establish();
    const pages = [];
    try {
        pages.push(
            await configuredPage(
                browser,
                baseURL,
                httpURL,
                config,
                peer.identity_id,
                conversation,
                errors,
                440,
            ),
        );
        pages.push(
            await configuredPage(
                browser,
                baseURL,
                httpURL,
                peer,
                config.identity_id,
                conversation,
                errors,
                660,
            ),
        );
        const [first, second] = pages;
        const rejected = await invite(first, second, false);
        await first.locator("#join-video").click();
        await first.waitForFunction(
            () =>
                document.querySelector("#state").textContent ===
                "Action failed",
        );
        assert.equal(
            await first.evaluate(() => globalThis.fixtureCaptureCount),
            0,
        );
        assert.equal(
            (await client.sessionGet(conversation, rejected)).state,
            "rejected",
        );
        const accepted = await invite(first, second, true);
        await Promise.all(
            pages.map((page) => click(page, "join-video", "Video connected")),
        );
        const initial = await Promise.all(
            pages.map((page, index) => decoded(page, index === 0 ? 660 : 440)),
        );
        await continuity.decide(accepted);
        for (const page of pages) {
            await page.locator("#mute-camera").click();
            assert.equal(
                await page.evaluate(
                    () =>
                        document
                            .querySelector("#local-video")
                            .srcObject.getVideoTracks()[0].enabled,
                ),
                false,
            );
            await page.locator("#mute-microphone").click();
            assert.equal(
                await page.evaluate(
                    () =>
                        document
                            .querySelector("#local-video")
                            .srcObject.getAudioTracks()[0].enabled,
                ),
                false,
            );
            await page.locator("#mute-camera").click();
            await page.locator("#mute-microphone").click();
        }
        await click(first, "end", "Session ended");
        await Promise.all(pages.map(reset));
        assert.equal(
            (await client.sessionGet(conversation, accepted)).state,
            "ended",
        );
        // The recipient creates a new AV Session in a different permitted Conversation.
        const next = await peerClient.create(
            "reference-video-next-conversation",
            [config.identity_id],
        );
        for (const page of pages)
            await page.locator("#conversation").fill(next.id);
        const fresh = await invite(second, first, true);
        assert.notEqual(fresh, accepted);
        await Promise.all(
            pages.map((page) => click(page, "join-video", "Video connected")),
        );
        const rejoined = await Promise.all(
            pages.map((page, index) => decoded(page, index === 0 ? 660 : 440)),
        );
        await click(second, "end-video", "Video left");
        await Promise.all(pages.map(reset));
        assert.equal(
            (await peerClient.sessionGet(next.id, fresh)).state,
            "accepted",
        );
        await peerClient.send(
            "reference-video-follow-up",
            next.id,
            "AV leg left; same conversation continues.",
        );
        assert(
            (await client.history(next.id)).messages.some(
                (item) =>
                    item.body === "AV leg left; same conversation continues.",
            ),
        );
        const failures = [];
        for (const failure of ["capture-ended", "ice-failed"]) {
            await Promise.all(
                pages.map((page) =>
                    click(page, "join-video", "Video connected"),
                ),
            );
            await Promise.all(
                pages.map((page, index) =>
                    decoded(page, index === 0 ? 660 : 440),
                ),
            );
            const began = performance.now();
            await first.evaluate((failure) => {
                if (failure === "capture-ended") {
                    const camera = document
                        .querySelector("#local-video")
                        .srcObject.getVideoTracks()[0];
                    camera.stop();
                    // MediaStreamTrack.stop() intentionally emits no ended event.
                    // Inject the event delivered by real capture-source loss.
                    camera.dispatchEvent(new Event("ended"));
                } else {
                    const connection = globalThis.fixturePeers.at(-1);
                    // Controlled ICE state fault on the actual negotiated peer;
                    // this qualifies cleanup, not an ICE network-timeout timer.
                    Object.defineProperty(connection, "iceConnectionState", {
                        value: "failed",
                    });
                    connection.dispatchEvent(
                        new Event("iceconnectionstatechange"),
                    );
                }
            }, failure);
            await Promise.all(pages.map(reset));
            const elapsedMs = performance.now() - began;
            assert(elapsedMs <= 5000);
            assert.equal(
                (await peerClient.sessionGet(next.id, fresh)).state,
                "accepted",
            );
            failures.push({
                failure,
                cleanupMs: elapsedMs,
                scope: "controlled source/state fault on actual negotiated WebRTC peers",
            });
        }
        await restartServer();
        assert.equal(
            (await client.sessionGet(conversation, accepted)).state,
            "ended",
        );
        const restored = await client.contextView(conversation, 100);
        const fact = restored.items.find(
            (item) =>
                item.body === "Written AV context: the chosen color is violet.",
        );
        assert(
            fact &&
                fact.actor_id === config.identity_id &&
                fact.source_id &&
                Number.isInteger(fact.sequence),
        );
        assert(
            restored.items.some(
                (item) =>
                    item.kind === "artifact_ready" &&
                    item.source_id === artifactId,
            ),
        );
        const ended = restored.items.find(
            (item) =>
                item.kind === "session_ended" && item.source_id === accepted,
        );
        assert(
            ended &&
                JSON.parse(ended.body).conversation_id === conversation &&
                JSON.parse(ended.body).state === "ended",
        );
        await peerClient.send(
            "reference-context-after-restart",
            conversation,
            "Continuing the violet decision after AV and restart.",
        );
        assert(
            (await client.contextView(conversation, 100)).items.some(
                (item) =>
                    item.body ===
                    "Continuing the violet decision after AV and restart.",
            ),
        );
        const continuityProof = await continuity.afterRestart(next.id);
        for (const [index, page] of pages.entries()) {
            await page.locator("#credential").fill("");
            await page.screenshot({
                path: path.join(scratch, `reference-video-${index}.png`),
                fullPage: true,
            });
            assert.equal(
                await page.evaluate(() => globalThis.fixtureCaptureCount),
                4,
            );
        }
        assert.deepEqual(errors, []);
        const proof = {
            status: "passed",
            scope: "real reference UI generated AV lifecycle; not sixty-second media qualification",
            checks: [
                "reject before capture",
                "selected local devices reach capture constraints",
                "public invite/accept and duplex decode",
                "independent camera/microphone controls",
                "explicit Session end releases both pages",
                "reverse-initiated fresh Session in new Conversation without restart",
                "leaving a leg retains accepted Session and Conversation",
                "follow-up message in same context",
                "attributed message, artifact and terminal Session context survive server restart",
                "capture loss and ICE failure release both legs within five seconds",
                "all owned tracks stopped",
                "source-backed decision during decoded AV supports one attributed follow-up after restart",
                "artifact bytes reverified after restart and wrong-conversation access denied",
            ],
            initial,
            continuity: continuityProof,
            rejoined,
            failures,
        };
        fs.writeFileSync(
            path.join(scratch, "reference-video.json"),
            JSON.stringify(proof, null, 2) + "\n",
            { mode: 0o600 },
        );
        return proof;
    } finally {
        await Promise.allSettled(pages.map((page) => page.close()));
    }
}
