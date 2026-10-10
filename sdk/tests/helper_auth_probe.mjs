// Actual Native SDK; known auth failure must not imply unknown send acceptance.
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
const { NativeClient, ConnectError } = await import(
    pathToFileURL(process.env.CONNECT_TYPESCRIPT_SDK).href
);
const [validConfig, badConfig, absentConfig, executable] =
    process.argv.slice(2);
let input = "";
for await (const chunk of process.stdin) input += chunk;
const { task, expires_at: expiresAt } = JSON.parse(input);
const reject = (fn, expected) =>
    assert.rejects(
        fn,
        (error) => error instanceof ConnectError && error.code === expected,
    );
for (const [config, expected] of [
    [badConfig, "unauthorized"],
    [absentConfig, "configuration"],
]) {
    const client = new NativeClient(config, executable);
    try {
        await reject(() => client.negotiateTask(task), expected);
        await reject(() => client.negotiateTask(task), expected);
    } finally {
        await client.close();
    }
}
let client = new NativeClient(validConfig, executable);
try {
    assert.equal((await client.negotiateTask(task)).id, task);
    // Exercise expiry on a live peer, rather than QUIC's earlier idle timeout.
    while (expiresAt * 1000 - Date.now() > 5000) {
        await new Promise((resolve) => setTimeout(resolve, 5000));
        if (expiresAt * 1000 - Date.now() > 5000) {
            const result = await client.task({ action: "get", task_id: task });
            assert.equal(result.kind, "record");
            assert.equal(result.data.id, task);
        }
    }
    await new Promise((resolve) =>
        setTimeout(resolve, Math.max(0, expiresAt * 1000 - Date.now()) + 1000),
    );
    await reject(
        () => client.task({ action: "get", task_id: task }),
        "unauthorized",
    );
} finally {
    await client.close();
}
client = new NativeClient(validConfig, executable);
try {
    await reject(() => client.negotiateTask(task), "unauthorized");
} finally {
    await client.close();
}
console.log(JSON.stringify({ status: "ok", runtime: "typescript" }));
