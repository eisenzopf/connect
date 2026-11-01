// Curated V8 sample attribution. Raw profiles and absolute source URLs remain private.
import { createHash } from "node:crypto";
const fail = () => { throw Error("Invalid bounded CPU observation"); };
const nativeNames = new Set(["writev", "writeBuffer", "writeUtf8String", "writeString", "read", "readStart", "readStop",
    "shutdown", "close", "open", "fstat", "stat", "fsync", "fdatasync", "spawnSync", "spawn", "prepare", "run",
    "all", "get", "exec", "post", "parse", "stringify", "fillRandom", "generateKeyPair", "verifyOneShot", "signOneShot", "getaddrinfo"]);
export function summarizeCpuProfile(bytes) {
    if (!(bytes instanceof Uint8Array) || bytes.length > 8 * 1024 * 1024) fail();
    const value = JSON.parse(Buffer.from(bytes).toString("utf8"));
    if (!Array.isArray(value.nodes) || value.nodes.length > 50000 ||
        !Array.isArray(value.samples) || value.samples.length > 150000 ||
        !Array.isArray(value.timeDeltas) || value.timeDeltas.length !== value.samples.length) fail();
    const nodes = new Map(), parents = new Map(), buckets = new Map();
    function label(frame) {
        if (!frame || typeof frame.functionName !== "string" || typeof frame.url !== "string") fail();
        let module = "other", name = "other", line = 0;
        for (const [prefix, title] of [["/consumer/app/", "app/"], ["/node_modules/@rudeless/connect/dist/", "sdk/"]]) {
            const position = frame.url.lastIndexOf(prefix), leaf = frame.url.slice(position + prefix.length);
            if (position >= 0 && /^[a-zA-Z0-9_-]+\.(?:mjs|js)$/.test(leaf)) module = title + leaf;
        }
        if (frame.url.startsWith("node:") && /^node:[a-zA-Z0-9_./-]{1,160}$/.test(frame.url)) module = frame.url;
        if (!frame.url && ["(root)", "(idle)", "(program)", "(garbage collector)"].includes(frame.functionName)) module = "v8";
        if (!frame.url && nativeNames.has(frame.functionName)) module = "node-native";
        if (module !== "other") {
            name = /^[a-zA-Z0-9_$# .<>()-]{0,96}$/.test(frame.functionName) ? frame.functionName || "anonymous" : "other";
            line = Number.isInteger(frame.lineNumber) && frame.lineNumber >= 0 && frame.lineNumber < 100000 ? frame.lineNumber + 1 : 0;
        }
        return { module, function: name, line };
    }
    for (const node of value.nodes) {
        if (!Number.isSafeInteger(node.id) || nodes.has(node.id) || !Array.isArray(node.children ?? [])) fail();
        const identity = label(node.callFrame), key = JSON.stringify(identity);
        nodes.set(node.id, key);
        if (!buckets.has(key)) buckets.set(key, identity);
        for (const child of node.children ?? []) { if (parents.has(child)) fail(); parents.set(child, node.id); }
    }
    let total = 0;
    for (const duration of value.timeDeltas) {
        if (!Number.isSafeInteger(duration) || duration < 0 || duration > 180000000) fail();
        total += duration;
    }
    function attribute(from) {
        const rows = new Map([...buckets].map(([key, identity]) => [key, { ...identity, self: 0, inclusive: 0 }]));
        let cursor = 0, samples = 0;
        for (let i = 0; i < value.samples.length; i++) {
            let id = value.samples[i];
            if (!nodes.has(id)) fail();
            const end = cursor + value.timeDeltas[i], duration = Math.max(0, end - Math.max(cursor, from)); cursor = end;
            if (duration) samples++;
            rows.get(nodes.get(id)).self += duration;
            const seen = new Set(), attributed = new Set();
            while (id !== undefined) {
                if (seen.has(id) || seen.size >= 256 || !nodes.has(id)) fail();
                seen.add(id);
                const key = nodes.get(id);
                if (!attributed.has(key)) { rows.get(key).inclusive += duration; attributed.add(key); }
                id = parents.get(id);
            }
        }
        const table = [...rows.values()].map(({ self, inclusive, ...identity }) => ({ ...identity,
            self_sampled_ms: Math.round(self / 1000), inclusive_sampled_ms: Math.round(inclusive / 1000) }));
        return { samples, total_sampled_ms: Math.round((total - from) / 1000),
            top_self: table.sort((a, b) => b.self_sampled_ms - a.self_sampled_ms).slice(0, 24),
            top_inclusive: [...table].sort((a, b) => b.inclusive_sampled_ms - a.inclusive_sampled_ms).slice(0, 24) };
    }
    const whole = attribute(0), tail = attribute(Math.max(0, total - 4000000));
    let observation;
    if (value.connect_observation !== undefined) {
        const v = value.connect_observation;
        if (!v || !["shutdown", "bound", "media-control-supervision"].includes(v.stop_reason) ||
            !["elapsed_us", "process_cpu_user_us", "process_cpu_system_us"].every(key =>
                Number.isSafeInteger(v[key]) && v[key] >= 0 && v[key] <= 60000000000)) fail();
        observation = { stop_reason: v.stop_reason, elapsed_ms: Math.round(v.elapsed_us / 1000),
            process_cpu_user_ms: Math.round(v.process_cpu_user_us / 1000), process_cpu_system_ms: Math.round(v.process_cpu_system_us / 1000) };
    }
    return { format: "connect/app-cpu-sample-summary/1", raw_profile_sha256: createHash("sha256").update(bytes).digest("hex"),
        ...whole, samples: value.samples.length, nodes: nodes.size, tail_four_seconds: tail, ...(observation ? { observation } : {}),
        scope: "Bounded Node/V8 sampling only. Application/SDK basenames and builtin frames; absolute URLs, values and unknown names excluded. Sample time is not a Rust/server CPU measurement or proof of timeout causation." };
}
