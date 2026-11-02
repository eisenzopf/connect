// Numeric summary of existing private Core trace. No payloads or target IDs escape.
import { readFile, readdir, lstat } from "node:fs/promises";
import { join } from "node:path";
import { createHash } from "node:crypto";
const MAX_BYTES = 8 * 1024 * 1024, MAX_LINES = 100000, MAX_TARGETS = 64;
const natural = value => /^(0|[1-9][0-9]{0,15})$/.test(value) && Number.isSafeInteger(Number(value));
const empty = () => ({ queued: [], io: [], refused: 0, trimmed: 0 });
function distribution(values) {
    const sorted = [...values].sort((a, b) => a - b);
    const percentile = p => sorted[Math.max(0, Math.ceil(sorted.length * p) - 1)] ?? null;
    return { samples: sorted.length, p50_us: percentile(.5), p95_us: percentile(.95), max_us: sorted.at(-1) ?? null };
}
export function summarizeNativeAudioTrace(text) {
    if (typeof text !== "string" || Buffer.byteLength(text) > MAX_BYTES || text.includes("\0")) throw Error("invalid-native-audio-trace");
    const lines = text.split("\n");
    if (lines.length > MAX_LINES || lines.some(line => Buffer.byteLength(line) > 16384)) throw Error("invalid-native-audio-trace");
    const totals = empty(), targets = new Map(); let malformed = 0, omittedTargets = 0;
    for (const original of lines) {
        const line = original.replace(/\x1b\[[0-9;]*m/g, "");
        if (!line.includes("rvoip_core::audio_path")) continue;
        const queued = /\bGuarded audio queued\b/.test(line), io = /\bGuarded audio entering IO\b/.test(line);
        if (queued || io) {
            const value = /\bage_us=([^\s]+)/.exec(line)?.[1];
            const target = /\btarget=([A-Za-z0-9_.:~-]{1,128})(?=\s|$)/.exec(line)?.[1];
            if (queued === io || !value || !natural(value) || !target) { malformed++; continue; }
            const field = queued ? "queued" : "io", age = Number(value); totals[field].push(age);
            if (!targets.has(target)) {
                if (targets.size >= MAX_TARGETS) { omittedTargets++; continue; }
                targets.set(target, empty());
            }
            targets.get(target)[field].push(age);
        } else if (/\bGuarded audio delivery refused\b/.test(line)) totals.refused++;
        else if (/\bGuarded audio pending burst trimmed\b/.test(line)) {
            const value = /\bdiscarded_chunks=([^\s]+)/.exec(line)?.[1];
            if (!value || !natural(value) || !Number.isSafeInteger(totals.trimmed + Number(value))) malformed++;
            else totals.trimmed += Number(value);
        }
    }
    const project = value => ({ mixer_queued_age: distribution(value.queued), entering_io_age: distribution(value.io) });
    return { format: "connect/private-native-audio-trace-summary/1", observed: totals.queued.length > 0 && totals.io.length > 0,
        ...project(totals), targets: [...targets.values()].map((value, targetSlot) => ({ target_slot: targetSlot, ...project(value) })),
        refused_events: totals.refused, discarded_chunks: totals.trimmed, malformed_events: malformed, omitted_target_events: omittedTargets,
        input_bytes: Buffer.byteLength(text), raw_logs_published: false,
        scope: "Existing Core trace ages from original source admission to mixer queue and guarded local IO. Samples are independent distributions, not paired packet transit, remote receive/decode/presentation, network loss, or latency qualification. Logging changes observation overhead; ordinary media verification remains separate." };
}

// Call only after the original app/server group has joined and flushed its logs.
export async function collectNativeAudioTrace(directory) {
    const info = await lstat(directory);
    if (!info.isDirectory() || info.isSymbolicLink() || (info.mode & 0o077)) throw Error("invalid-native-audio-log-directory");
    const names = (await readdir(directory)).filter(name => /^[0-9a-f-]{36}-[0-9]+-server\.log$/.test(name)).sort();
    if (names.length > 4) throw Error("invalid-native-audio-log-count");
    const files = [];
    for (const name of names) {
        const path = join(directory, name), stat = await lstat(path);
        if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1 || (stat.mode & 0o077) || stat.size > MAX_BYTES) throw Error("invalid-native-audio-log");
        const bytes = await readFile(path);
        if (bytes.length !== stat.size) throw Error("native-audio-log-changed");
        const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        files.push({ server_slot: files.length, private_log_sha256: createHash("sha256").update(bytes).digest("hex"), ...summarizeNativeAudioTrace(text) });
    }
    return { observed: files.some(file => file.observed), files, raw_logs_published: false,
        scope: "Post-join numeric summaries of this fixture's private server logs. No identifiers, absolute paths or raw log lines. Diagnostic logging affects overhead and does not qualify ordinary media latency." };
}
