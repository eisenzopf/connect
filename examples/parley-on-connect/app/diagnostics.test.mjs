import test from "node:test";
import assert from "node:assert/strict";
import { appDiagnostic } from "./diagnostics.mjs";
test("backend diagnostics retain only closed failure classes and source locations", () => {
    const error = Object.assign(Error("private-token-and-body"), { code: "invalid-app-intent", kind: "validation", credential: "private-token-and-body" });
    error.stack = "Error: private-token-and-body\n at read (/private/backend/account-name/app/gateway.mjs:123:4)";
    assert.deepEqual(appDiagnostic(error), { code: "invalid-app-intent", name: "Error", kind: "validation", location: { file: "gateway.mjs", line: 123, column: 4 }, sdk_location: null });
    assert.ok(!JSON.stringify(appDiagnostic(error)).includes("private"));
    assert.deepEqual(appDiagnostic({ code: "private-token-and-body", name: "private-label", kind: "private-reason" }),
        { code: null, name: null, kind: null, location: null, sdk_location: null });
});
test("SDK diagnostics retain only known module coordinates without a private path or exception message", () => {
    const error = Error("private-credential-and-body");
    error.stack = "Error: private-credential-and-body\n at decode (/private/account/node_modules/@rudeless/connect/dist/node_media.js:210:7)";
    const value = appDiagnostic(error);
    assert.deepEqual(value.sdk_location, { file: "node_media.js", line: 210, column: 7 });
    assert.ok(!JSON.stringify(value).includes("private"));
    error.stack = error.stack.replace("node_media.js", "private_secret.js");
    assert.equal(appDiagnostic(error).sdk_location, null);
});


test("media diagnostics admit only fixed operation phases and omit arbitrary context", () => {
    const error = Object.assign(Error("private message"), { kind: "cancelled", name: "NodeSdkError" });
    const safe = appDiagnostic(error, { phase: "open-native-negotiation", credential: "private-token" });
    assert.equal(safe.phase, "open-native-negotiation"); assert.ok(!JSON.stringify(safe).includes("private"));
    const unknown = appDiagnostic(error, { phase: "private-token" });
    assert.equal(Object.hasOwn(unknown, "phase"), false);
});

test("media read timing admits only bounded integers in a known phase without copying error properties", () => {
    const error = Object.assign(Error("private body"), { elapsedMs: "private-token" });
    const value = appDiagnostic(error, { phase: "media-control-supervision", elapsedMs: 2001,
        totalElapsedMs: 2010, eventLoopActiveMs: 3, eventLoopIdleMs: 1998, cpuUserMs: 2, cpuSystemMs: 1, credential: "private-token" });
    assert.deepEqual([value.elapsed_ms, value.total_elapsed_ms, value.event_loop_active_ms, value.event_loop_idle_ms], [2001, 2010, 3, 1998]);
    assert.deepEqual([value.process_cpu_user_ms, value.process_cpu_system_ms], [2, 1]);
    assert.ok(!JSON.stringify(value).includes("private"));
    for (const bad of [-1, 60001, NaN, Infinity, 1.5, "private-token", {}]) {
        const rejected = appDiagnostic(error, { phase: "open-current-snapshot", elapsedMs: bad,
            totalElapsedMs: bad, eventLoopActiveMs: bad, eventLoopIdleMs: bad, cpuUserMs: bad, cpuSystemMs: bad });
        assert.ok(Object.keys(rejected).every(key => !key.endsWith("_ms")));
    }
    assert.ok(Object.keys(appDiagnostic(error, { phase: "private-token", elapsedMs: 2 })).every(key => !key.endsWith("_ms")));
});
