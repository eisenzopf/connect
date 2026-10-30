import { mkdir, lstat, open, readFile } from "node:fs/promises";
import { isAbsolute, resolve, sep, join } from "node:path";
import { digest } from "./policy.mjs";
import { stateJson } from "./state.mjs";
const fail = code => { const error = Error(code); error.code = code; throw error; };
const copy = value => JSON.parse(stateJson(value));
async function syncDirectory(path) { const handle = await open(path, "r"); try { await handle.sync(); } finally { await handle.close(); } }
async function exclusiveJson(path, value) {
    const handle = await open(path, "wx", 0o600);
    try { await handle.writeFile(stateJson(value) + "\n"); await handle.sync(); }
    finally { await handle.close(); }
}
async function privateJson(path) {
    const info = await lstat(path);
    if (!info.isFile() || (info.mode & 0o077) !== 0 || info.size > 65536) fail("private-model-record-required");
    let value;
    try { value = JSON.parse(await readFile(path, "utf8")); stateJson(value); }
    catch { fail("corrupt-model-record"); }
    return value;
}
/** Durable boundary around a trusted proposal-only model callback. It owns no
 * Connect client/credential and provides no provider communications API. The
 * caller must separately authorize/configure any actual model execution.
 * recover() reads the original result; it never invokes or resumes a model. */
export class RecordedProposalModel {
    #directory; #invoke; #source; #observe;
    constructor({ directory, source, invoke, recoverOriginal }) {
        if (!isAbsolute(directory) || !resolve(directory).split(sep).includes(".local") ||
            typeof invoke !== "function" || !source || typeof source.mode !== "string") fail("invalid-model-adapter");
        if (recoverOriginal !== undefined && typeof recoverOriginal !== "function") fail("invalid-model-adapter");
        this.#directory = directory; this.#invoke = invoke; this.#source = copy(source); this.#observe = recoverOriginal;
    }
    async #path(request, create) {
        if (!/^model-[a-f0-9]{48}$/.test(request.requestId) || request.output !== null ||
            Buffer.byteLength(stateJson(request)) > 40000) fail("invalid-original-model-request");
        await mkdir(this.#directory, { recursive: true, mode: 0o700 });
        const parent = await lstat(this.#directory);
        if (!parent.isDirectory() || (parent.mode & 0o077) !== 0) fail("private-model-directory-required");
        const directory = join(this.#directory, request.requestId);
        let created = false;
        if (create) {
            try { await mkdir(directory, { mode: 0o700 }); created = true; }
            catch (error) { if (error.code !== "EEXIST") throw error; }
        }
        try {
            const info = await lstat(directory);
            if (!info.isDirectory() || (info.mode & 0o077) !== 0) fail("private-model-directory-required");
        } catch (error) { if (!create && error.code === "ENOENT") return null; throw error; }
        const original = { profile: "parley-model-request/1", request: copy(request), source: this.#source };
        if (created) {
            await exclusiveJson(join(directory, "request.json"), original);
            await syncDirectory(directory); await syncDirectory(this.#directory);
        } else {
            let saved;
            try { saved = await privateJson(join(directory, "request.json")); }
            catch (error) { if (error.code === "ENOENT") fail("incomplete-model-request"); throw error; }
            if (digest(saved) !== digest(original)) fail("different-original-model-request");
        }
        return { directory, created, requestDigest: digest(original) };
    }
    async generate(request) {
        const original = copy(request), owned = await this.#path(original, true);
        if (!owned.created) {
            const recovered = await this.recover(original);
            if (recovered.state !== "completed") fail("original-model-request-pending");
            return recovered.output;
        }
        // request.json is fsynced before any callback can execute a model.
        // Callback errors or process exit retain this directory, preventing a
        // second generate() call from concealing an uncertain original turn.
        const result = copy(await this.#invoke(copy(original)));
        if (Buffer.byteLength(stateJson(result)) > 8192) fail("model-result-capacity");
        await exclusiveJson(join(owned.directory, "response.json"), {
            profile: "parley-model-response/1", requestDigest: owned.requestDigest, output: result,
        });
        await syncDirectory(owned.directory);
        return copy(result);
    }
    async recover(request) {
        const owned = await this.#path(copy(request), false);
        if (!owned) return { state: "unknown" };
        let result;
        try { result = await privateJson(join(owned.directory, "response.json")); }
        catch (error) {
            if (error.code !== "ENOENT") throw error;
            if (!this.#observe) return { state: "unknown" };
            // Trusted app callback observes the original provider/host only.
            // It must never generate/resume a replacement model request.
            const observed = await this.#observe(copy(request));
            if (observed.state !== "completed") return { state: "unknown" };
            const output = copy(observed.output);
            if (Buffer.byteLength(stateJson(output)) > 8192) fail("model-result-capacity");
            result = { profile: "parley-model-response/1", requestDigest: owned.requestDigest, output };
            try { await exclusiveJson(join(owned.directory, "response.json"), result); await syncDirectory(owned.directory); }
            catch (writeError) {
                if (writeError.code !== "EEXIST") throw writeError;
                const existing = await privateJson(join(owned.directory, "response.json"));
                if (digest(existing) !== digest(result)) fail("different-model-response");
            }
        }
        if (result.profile !== "parley-model-response/1" || result.requestDigest !== owned.requestDigest ||
            !result.output || Buffer.byteLength(stateJson(result.output)) > 8192) fail("different-model-response");
        return { state: "completed", output: copy(result.output) };
    }
}

/** No account/model/provider access. Explicitly simulated proposals share the
 * same durable request and worker validation path as a reviewed external host. */
export function scriptedModel({ directory, responses }) {
    const script = copy(responses);
    return new RecordedProposalModel({ directory,
        source: { mode: "scripted", simulated: true, scriptDigest: digest(script) },
        invoke: async request => {
            const result = script[request.input.stage];
            if (!result) fail("missing-scripted-proposal");
            return copy(result);
        } });
}
