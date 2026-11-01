import test from "node:test";
import assert from "node:assert/strict";
import { appDiagnostic } from "./diagnostics.mjs";
test("backend diagnostics retain only closed failure classes and source locations", () => {
    const error = Object.assign(Error("private-token-and-body"), { code: "invalid-app-intent", kind: "validation", credential: "private-token-and-body" });
    error.stack = "Error: private-token-and-body\n at read (/private/backend/account-name/app/gateway.mjs:123:4)";
    assert.deepEqual(appDiagnostic(error), { code: "invalid-app-intent", name: "Error", kind: "validation", location: { file: "gateway.mjs", line: 123, column: 4 } });
    assert.ok(!JSON.stringify(appDiagnostic(error)).includes("private"));
    assert.deepEqual(appDiagnostic({ code: "private-token-and-body", name: "private-label", kind: "private-reason" }),
        { code: null, name: null, kind: null, location: null });
});
