// Explicit isolated browser SDP fixture; no Connect server/provider or media proof.
import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { resolve, sep, join } from "node:path";
import { pathToFileURL } from "node:url";
const output = resolve(process.argv[2] ?? "");
assert(output.split(sep).includes(".local") && process.argv[2]);
assert(process.env.CONNECT_PLAYWRIGHT_MODULE && process.env.CONNECT_CHROME_BINARY);
await mkdir(output, { mode: 0o700 }); // Never replace an earlier fixture.
const { chromium } = await import(pathToFileURL(resolve(process.env.CONNECT_PLAYWRIGHT_MODULE)).href);
let browser;
try {
    browser = await chromium.launch({ headless: true, executablePath: process.env.CONNECT_CHROME_BINARY });
    const page = await browser.newPage();
    const result = await page.evaluate(async () => {
        const results = [];
        async function gather(peer) {
            if (peer.iceGatheringState === "complete") return;
            await new Promise((resolve, reject) => {
                const timer = setTimeout(() => { peer.removeEventListener("icegatheringstatechange", changed); reject(Error("Gather deadline")); }, 8000);
                function changed() {
                    if (peer.iceGatheringState === "complete") {
                        clearTimeout(timer); peer.removeEventListener("icegatheringstatechange", changed); resolve();
                    }
                }
                peer.addEventListener("icegatheringstatechange", changed); changed();
            });
        }
        for (const kinds of [["audio"], ["audio", "video"]]) {
            const a = new RTCPeerConnection({ iceServers: [] }), b = new RTCPeerConnection({ iceServers: [] });
            const observation = { profile: kinds.join("+"), closed: false };
            try {
                for (const kind of kinds) a.addTransceiver(kind, { direction: "sendrecv" });
                await a.setLocalDescription(await a.createOffer()); await gather(a);
                observation.original = a.localDescription.sdp;
                await b.setRemoteDescription(a.localDescription);
                await b.setLocalDescription(await b.createAnswer()); await gather(b);
                await a.setRemoteDescription(b.localDescription);
                await a.setLocalDescription(await a.createOffer({ iceRestart: true })); await gather(a);
                observation.proposed = a.localDescription.sdp;
                observation.signaling = a.signalingState;
            } finally {
                a.close(); b.close(); observation.closed = a.signalingState === "closed" && b.signalingState === "closed";
            }
            results.push(observation);
        }
        return results;
    });
    for (const scene of result) {
        assert(scene.closed && scene.signaling === "have-local-offer");
        for (const key of ["original", "proposed"])
            await writeFile(join(output, scene.profile + "-" + key + ".sdp"), scene[key], { flag: "wx", mode: 0o600 });
    }
    const summary = { browser: browser.version(), scope: "Actual local browser-generated initial and restart SDP only; no server restart, decoded media, remote/relay or ICE-readiness qualification",
        profiles: result.map(({ profile, closed, signaling }) => ({ profile, closed, signaling })) };
    await writeFile(join(output, "summary.json"), JSON.stringify(summary, null, 2) + "\n", { flag: "wx", mode: 0o600 });
    console.log(JSON.stringify(summary));
} finally {
    if (browser) await browser.close();
}
