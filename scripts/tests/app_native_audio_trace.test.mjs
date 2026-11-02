import assert from "node:assert/strict";
import test from "node:test";
import { summarizeNativeAudioTrace, collectNativeAudioTrace } from "../lib/app_native_audio_trace.mjs";
import { mkdtemp, writeFile, symlink, chmod, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
const event = (message, target, age) => `2026-11-02T20:00:00Z TRACE rvoip_core::audio_path: ${message} target=${target} age_us=${age}`;
test("mixed private targets and ANSI traces produce numeric independent distributions only", () => {
    const result = summarizeNativeAudioTrace([event("Guarded audio queued", "private-alpha", 1000),
        event("Guarded audio entering IO", "private-alpha", 3000),
        "\x1b[32m" + event("Guarded audio queued", "private-beta", 4000) + "\x1b[0m",
        event("Guarded audio entering IO", "private-beta", 8000),
        "TRACE rvoip_core::audio_path: Guarded audio delivery refused source=secret-source problem=secret-problem",
        "TRACE rvoip_core::audio_path: Guarded audio pending burst trimmed source=secret-source discarded_chunks=2"].join("\n"));
    assert.equal(result.observed, true); assert.equal(result.targets.length, 2);
    assert.deepEqual(result.mixer_queued_age, { samples: 2, p50_us: 1000, p95_us: 4000, max_us: 4000 });
    assert.equal(result.entering_io_age.max_us, 8000); assert.equal(result.refused_events, 1); assert.equal(result.discarded_chunks, 2);
    assert.doesNotMatch(JSON.stringify(result), /private-alpha|private-beta|secret-source|secret-problem|2026/);
});
test("unrelated output, unknown messages and malformed ages cannot invent an observation", () => {
    const lines = ["arbitrary secret payload", "TRACE other: Guarded audio queued target=secret age_us=1",
        "TRACE rvoip_core::audio_path: unknown-message secret=payload",
        ...["-1", "01", "1.5", "NaN", "9007199254740992"].map(age => event("Guarded audio queued", "secret", age)),
        "TRACE rvoip_core::audio_path: Guarded audio entering IO age_us=1"];
    const result = summarizeNativeAudioTrace(lines.join("\n"));
    assert.equal(result.observed, false); assert.equal(result.malformed_events, 6);
    assert.equal(result.entering_io_age.samples, 0); assert.equal(result.mixer_queued_age.p50_us, null);
    assert.doesNotMatch(JSON.stringify(result), /secret|payload|NaN/);
});
test("bounded parser rejects binary, overlong lines and oversized logs", () => {
    for (const value of [null, "binary\0data", "a".repeat(16385), "\n".repeat(100000), "x".repeat(8 * 1024 * 1024 + 1)])
        assert.throws(() => summarizeNativeAudioTrace(value), /invalid-native-audio-trace/);
});
test("target limit remains explicit while aggregate original sample counts are retained", () => {
    const result = summarizeNativeAudioTrace(Array.from({ length: 65 }, (_, n) => event("Guarded audio queued", "private-" + n, n)).join("\n"));
    assert.equal(result.targets.length, 64); assert.equal(result.omitted_target_events, 1);
    assert.equal(result.mixer_queued_age.samples, 65); assert.equal(result.observed, false);
});
test("closed original logs yield hashes and numeric summaries without exposing unrelated files", async () => {
    const directory = await mkdtemp(join(tmpdir(), "connect-native-audio-"));
    try {
        await chmod(directory, 0o700);
        const name = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa-1-server.log";
        await writeFile(join(directory, name), [event("Guarded audio queued", "private-target", 1234), event("Guarded audio entering IO", "private-target", 5678)].join("\n"), { mode: 0o600 });
        await writeFile(join(directory, "other-private.log"), "private-unrelated", { mode: 0o600 });
        const result = await collectNativeAudioTrace(directory);
        assert.equal(result.observed, true); assert.equal(result.files.length, 1);
        assert.match(result.files[0].private_log_sha256, /^[0-9a-f]{64}$/);
        assert.equal(result.files[0].entering_io_age.max_us, 5678);
        assert.doesNotMatch(JSON.stringify(result), /private-target|private-unrelated|aaaaaaaa|other-private|connect-native-audio-/);
        await chmod(join(directory, name), 0o644);
        await assert.rejects(collectNativeAudioTrace(directory), /invalid-native-audio-log/);
        await rm(join(directory, name)); await symlink(join(directory, "other-private.log"), join(directory, name));
        await assert.rejects(collectNativeAudioTrace(directory), /invalid-native-audio-log/);
    } finally { await rm(directory, { recursive: true }); }
});
