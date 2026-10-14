/** Isolated, offline Chromium proof of the actual reference UI and public HTTP client.
 * Supply CONNECT_PLAYWRIGHT_MODULE / CONNECT_CHROME_BIN when not installed locally.
 * No provider worker, microphone, real credentials, or retained installation is used.
 */
import { createServer } from "node:http";
import { once } from "node:events";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createHash } from "node:crypto";
import assert from "node:assert/strict";
import { referenceInputSnapshot } from "./lib/reference_inputs.mjs";
import { ReferenceRuntime } from "./lib/reference_runtime.mjs";
import { qualifyReferenceVideo } from "./lib/reference_video.mjs";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const assets = path.resolve(process.env.CONNECT_TEST_RUNTIME_ROOT || root);
const scratchRoot = path.join(root, ".local/gate5/reference");
fs.mkdirSync(scratchRoot, { recursive: true, mode: 0o700 });
const scratch = fs.mkdtempSync(path.join(scratchRoot, "browser-"));
const environment = Object.fromEntries(
    ["PATH", "HOME", "USER", "LANG", "TMPDIR"]
        .filter((key) => process.env[key])
        .map((key) => [key, process.env[key]]),
);
environment.PYTHONDONTWRITEBYTECODE = "1";
fs.chmodSync(scratch, 0o700);
const executable = path.resolve(
    process.env.CONNECT_BINARY || path.join(root, "target/debug/connect"),
);
const inputsBefore = referenceInputSnapshot(assets, executable, process.env);
const began = performance.now();
const { chromium } = await import(
    process.env.CONNECT_PLAYWRIGHT_MODULE || "playwright"
);
const { HttpClient } = await import(
    pathToFileURL(path.join(assets, "sdk/typescript/dist/index.js")).href
);
const runtime = new ReferenceRuntime(executable, scratch, environment);
const cfg = runtime.config,
    peer = runtime.peer;
const url = await runtime.start();
let browser, staticServer, result;
try {
    const client = new HttpClient(url, cfg.credential);
    await runtime.ready(client);
    const conversation = await client.create("browser-check", [
        peer.identity_id,
    ]);
    for (let i = 0; i < 101; i++)
        await client.send(`seed-${i}`, conversation.id, `Seed message ${i}`);
    staticServer = createServer((req, res) => {
        const requested = new URL(req.url, "http://localhost").pathname;
        if (
            !["/examples/conversation-client/", "/sdk/typescript/dist/"].some(
                (prefix) => requested.startsWith(prefix),
            ) ||
            requested.includes("..")
        ) {
            res.writeHead(404);
            res.end();
            return;
        }
        const file = path.join(
            assets,
            requested.endsWith("/") ? requested + "index.html" : requested,
        );
        try {
            res.setHeader(
                "Content-Type",
                file.endsWith(".js")
                    ? "text/javascript"
                    : file.endsWith(".css")
                      ? "text/css"
                      : "text/html",
            );
            res.end(fs.readFileSync(file));
        } catch {
            res.writeHead(404);
            res.end();
        }
    });
    staticServer.listen(0, "127.0.0.1");
    await once(staticServer, "listening");
    browser = await chromium.launch({
        headless: true,
        args: [
            "--autoplay-policy=no-user-gesture-required",
            "--mute-audio",
            "--disable-background-timer-throttling",
            "--disable-renderer-backgrounding",
        ],
        ...(process.env.CONNECT_CHROME_BINARY || process.env.CONNECT_CHROME_BIN
            ? {
                  executablePath:
                      process.env.CONNECT_CHROME_BINARY ||
                      process.env.CONNECT_CHROME_BIN,
              }
            : {}),
    });
    const page = await browser.newPage({
        viewport: { width: 1400, height: 1000 },
    });
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(() => {
        globalThis.microphoneRequests = 0;
        navigator.mediaDevices.getUserMedia = async () => {
            globalThis.microphoneRequests++;
            throw Error("fixture forbids recording");
        };
    });
    await page.goto(
        `http://127.0.0.1:${staticServer.address().port}/examples/conversation-client/`,
    );
    async function connect() {
        for (const [id, value] of Object.entries({
            http: url,
            credential: cfg.credential,
            identity: cfg.identity_id,
            participant: peer.identity_id,
            conversation: conversation.id,
        }))
            await page.locator("#" + id).fill(value);
        await page.locator("#connect").click();
        await page.waitForFunction(
            () =>
                document.querySelector("#state").textContent === "Connected" &&
                document.querySelector("#detail").textContent === "Complete.",
        );
    }
    await connect();
    await page.locator("#refresh").click();
    await page.waitForFunction(
        () => document.querySelectorAll("#messages>li").length === 100,
    );
    await page.locator("#more-history").click();
    await page.waitForFunction(
        () => document.querySelectorAll("#messages>li").length === 101,
    );
    assert.match(
        await page.locator("#messages").innerText(),
        /Seed message 100/,
    );
    await page.locator("#message").fill("Keyboard reply proof");
    await page.locator("#message-form button").focus();
    await page.keyboard.press("Enter");
    await page.waitForFunction(
        () =>
            document.querySelector("#state").textContent === "Message sent" &&
            document.querySelector("#detail").textContent === "Complete.",
    );
    const bytes = Buffer.alloc(70000);
    for (let i = 0; i < bytes.length; i++) bytes[i] = i % 251;
    await page.locator("#file").setInputFiles({
        name: "reference.bin",
        mimeType: "application/octet-stream",
        buffer: bytes,
    });
    await page.locator("#upload").click();
    await page.waitForFunction(() =>
        document
            .querySelector("#file-state")
            .textContent.startsWith("Ready · "),
    );
    const artifactId = (await page.locator("#file-state").innerText()).split(
        " · ",
    )[1];
    await page.locator("#artifact-list button").last().click();
    assert.equal(await page.locator("#artifact-id").inputValue(), artifactId);
    const downloaded = page.waitForEvent("download");
    await page.locator("#download").click();
    const download = await downloaded;
    const downloadPath = path.join(scratch, "downloaded.bin");
    await download.saveAs(downloadPath);
    assert.deepEqual(fs.readFileSync(downloadPath), bytes);
    await page.waitForFunction(
        () =>
            document.querySelector("#state").textContent === "File verified" &&
            document.querySelector("#detail").textContent === "Complete.",
    );
    const routeFaults = [];
    const guardedRoute = (handler) => async (route) => {
        try {
            await handler(route);
        } catch (error) {
            routeFaults.push(error.name);
            await route.abort("failed").catch(() => {});
        }
    };
    let corrupt = true;
    await page.route(
        url + "/v1/operations",
        guardedRoute(async (route) => {
            if (
                route.request().postDataJSON().op === "artifact_get" &&
                corrupt
            ) {
                const response = await route.fetch(),
                    body = await response.json();
                assert.equal(body.result?.Ok?.kind, "artifact_chunk");
                const chunk = body.result.Ok.data;
                const changed = Buffer.from(chunk.data, "base64");
                changed[0] ^= 1;
                chunk.data = changed.toString("base64");
                await route.fulfill({ response, json: body });
                return;
            }
            await route.continue();
        }),
    );
    await page.locator("#download").click();
    await page.waitForFunction(
        () => document.querySelector("#state").textContent === "Action failed",
    );
    assert.deepEqual(routeFaults, []);
    assert.match(await page.locator("#detail").innerText(), /integrity/);
    corrupt = false;
    await page.unroute(url + "/v1/operations");
    const other = await client.create("other-file-context", [peer.identity_id]);
    await page.locator("#conversation").fill(other.id);
    await page.locator("#download").click();
    await page.waitForFunction(
        () => document.querySelector("#state").textContent === "Action failed",
    );
    assert.doesNotMatch(await page.locator("#detail").innerText(), /Verified/);
    await page.locator("#conversation").fill(conversation.id);
    const expiring = await client.artifactBegin(
        "expiring-ui-file",
        conversation.id,
        "expiring.txt",
        "text/plain",
        1,
        createHash("sha256").update("x").digest("hex"),
        Math.floor(Date.now() / 1000) + 2,
    );
    await client.artifactAppend(
        "expiring-ui-byte",
        conversation.id,
        expiring.id,
        0,
        Buffer.from("x").toString("base64"),
    );
    await client.artifactFinish(
        "expiring-ui-finish",
        conversation.id,
        expiring.id,
    );
    await new Promise((resolve) => setTimeout(resolve, 2200));
    await page.locator("#artifact-id").fill(expiring.id);
    await page.locator("#download").click();
    await page.waitForFunction(
        () => document.querySelector("#state").textContent === "Action failed",
    );
    assert.match(await page.locator("#detail").innerText(), /forbidden/);
    await page.setViewportSize({ width: 390, height: 844 });
    assert(
        await page.evaluate(
            () => document.documentElement.scrollWidth <= window.innerWidth,
        ),
        "narrow layout overflows",
    );
    await page.locator("#credential").fill("");
    await page.screenshot({
        path: path.join(scratch, "reference-mobile.png"),
        fullPage: true,
    });
    await page.setViewportSize({ width: 1400, height: 1000 });
    await page.locator("#credential").fill(cfg.credential);
    let lost = false,
        firstId;
    await page.route(
        url + "/v1/operations",
        guardedRoute(async (route) => {
            const op = route.request().postDataJSON();
            if (
                op.op === "send" &&
                op.body === "Lost acknowledgement proof" &&
                !lost
            ) {
                lost = true;
                firstId = op.request_id;
                await route.fetch();
                await route.abort("failed");
                return;
            }
            if (op.op === "send" && op.body === "Lost acknowledgement proof")
                assert.equal(op.request_id, firstId);
            await route.continue();
        }),
    );
    await page.locator("#message").fill("Lost acknowledgement proof");
    await page
        .locator("#message-form")
        .evaluate((form) => form.requestSubmit());
    await page.waitForFunction(
        () => document.querySelector("#state").textContent === "Action failed",
    );
    assert.deepEqual(routeFaults, []);
    await page.reload();
    await connect();
    await page.locator("#retry").click();
    await page.waitForFunction(
        () =>
            document.querySelector("#state").textContent ===
                "Saved action recovered" &&
            document.querySelector("#detail").textContent === "Complete.",
    );
    const first = await client.history(conversation.id, null, 100);
    const second = await client.history(conversation.id, first.next, 100);
    assert.equal(
        [...first.messages, ...second.messages].filter(
            (m) => m.body === "Lost acknowledgement proof",
        ).length,
        1,
    );
    await page.locator("#join").click();
    await page.waitForFunction(
        () => document.querySelector("#state").textContent === "Action failed",
    );
    assert.equal(await page.evaluate(() => globalThis.microphoneRequests), 0);
    await page.locator("#credential").fill("");
    await page.screenshot({
        path: path.join(scratch, "reference-client.png"),
        fullPage: true,
    });
    assert.deepEqual(errors, []);
    const video = await qualifyReferenceVideo({
        browser,
        baseURL: `http://127.0.0.1:${staticServer.address().port}`,
        httpURL: url,
        config: cfg,
        peer,
        conversation: conversation.id,
        artifactId,
        client,
        peerClient: new HttpClient(url, peer.credential),
        configureAndRestart: () => runtime.enableVideo(conversation.id, client),
        restartServer: async () => {
            await runtime.stop();
            await runtime.start();
            await runtime.ready(client);
        },
        scratch,
    });
    result = {
        video,
        result: "passed",
        runtime_root: assets,
        binary: executable,
        binary_sha256: createHash("sha256")
            .update(fs.readFileSync(executable))
            .digest("hex"),
        source_revision: process.env.CONNECT_SOURCE_REVISION ?? null,
        input_sha256: inputsBefore,
        whole_gate_qualified: false,
        scope: "offline real Chromium and local public HTTP interface",
        checks: [
            "101-message history paging",
            "lost acknowledgement followed by reload and same-ID replay",
            "exactly one committed message",
            "voice refusal before microphone acquisition",
            "keyboard message submission",
            "public UI upload/download with independent byte verification",
            "corrupted download rejected before save",
            "cross-conversation file denied",
            "expired file denied",
            "390px narrow layout",
            "no page errors",
        ],
        screenshot: "reference-client.png",
    };
} catch (error) {
    fs.writeFileSync(
        path.join(scratch, "failure-private.txt"),
        String(error.stack ?? error.name).slice(0, 1024 * 1024),
        { mode: 0o600 },
    );
    result = {
        result: "failed",
        error_class: error.name,
        scope: "offline real reference UI; no hardware/providers",
        whole_gate_qualified: false,
    };
    process.exitCode = 1;
} finally {
    const cleanup = await Promise.allSettled([
        browser ? browser.close() : Promise.resolve(),
        staticServer
            ? new Promise((resolve) => staticServer.close(resolve))
            : Promise.resolve(),
        runtime.close(),
    ]);
    const failedCleanup = cleanup.find((item) => item.status === "rejected");
    if (failedCleanup) {
        result = {
            ...result,
            result: "failed",
            cleanup_error_class: failedCleanup.reason.name,
        };
        process.exitCode = 1;
    }
    try {
        const inputsAfter = referenceInputSnapshot(
            assets,
            executable,
            process.env,
        );
        assert.deepEqual(
            inputsAfter,
            inputsBefore,
            "public reference inputs changed during proof",
        );
        result.tested_inputs_stable = true;
    } catch (error) {
        result = {
            ...result,
            result: "failed",
            tested_inputs_stable: false,
            input_error_class: error.name,
        };
        process.exitCode = 1;
    }
    result.elapsed_seconds = (performance.now() - began) / 1000;
    if (process.env.CONNECT_REFERENCE_OUTPUT)
        fs.writeFileSync(
            process.env.CONNECT_REFERENCE_OUTPUT,
            JSON.stringify(result, null, 2) + "\n",
            { mode: 0o600 },
        );
    fs.writeFileSync(
        path.join(scratch, "result.json"),
        JSON.stringify(result, null, 2) + "\n",
        { mode: 0o600 },
    );
    console.log(JSON.stringify({ ...result, evidence_directory: scratch }));
}
