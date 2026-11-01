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
