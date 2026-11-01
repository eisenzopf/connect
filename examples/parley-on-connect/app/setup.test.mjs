import test from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, writeFile, readFile, readdir, symlink } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { privateDirectory, privateBytes, writeExact, loadOrCreatePlan, configureActors, participantOperations, verifyBootstrap } from "./setup.mjs";

const root = fileURLToPath(new URL("../../../.local/gate-6/reference-app/setup-tests/", import.meta.url));
const binarySha = "a".repeat(64), ports = { native: 24431, http: 24432, ui: 24433 };
async function fixture() {
    await mkdir(root, { recursive: true, mode: 0o700 });
    const directory = await mkdtemp(join(root, "unit-"));
    await privateDirectory(join(directory, "installation"));
    // Explicit SDK-shaped/CLI-shaped setup fixture. These are synthetic private
    // bytes, not real Connect configurations or a backend qualification.
    for (const name of ["server.toml", "operator.toml", "operator.application.toml", "certificate.der", "private-key.der", "server-signing.der", "server-signing.pub"])
        await writeExact(join(directory, "installation", name), name === "operator.toml" ? 'credential = "fixture-admin"\n' : "fixture\n");
    const plan = await loadOrCreatePlan(directory, binarySha, ports), calls = [], identities = new Map(), credentials = new Map();
    let lostIdentity = false, lostIssue = false, lostVerification = false, nativeAllowed = true, changeDevice = false, adminCloses = 0, participantCloses = 0;
    const endpoints = new Map(), verifications = new Map();
    const createAdministrativeClient = path => {
        assert.equal(path, join(directory, "installation/operator.sdk.toml"));
        return {
            validateProvisioning: value => {
                if (value.type === "identity") assert.match(value.alias, /^[a-z0-9-]{1,64}$/);
                return { value };
            },
            provision: async (command, options) => {
                calls.push({ command: structuredClone(command.value), original: options.operationId });
                assert.equal(options.operationId, command.value.operation_id);
                const value = command.value;
                if (value.type === "endpoint") {
                    const old = endpoints.get(value.operation_id);
                    if (old) assert.deepEqual(old.command, value);
                    else endpoints.set(value.operation_id, { command: structuredClone(value), result: {
                        kind: "endpoint", endpoint_id: "endpoint_" + endpoints.size, identity_id: value.identity_id,
                        device_id: value.device.id, transport: value.transport, capabilities: value.capabilities,
                        revision: 0, verification_state: "unverified",
                    } });
                    return { wait: async () => ({ snapshot: { state: "completed" }, value: structuredClone(endpoints.get(value.operation_id).result) }) };
                }
                if (value.type === "verify_endpoint") {
                    const endpoint = [...endpoints.values()].find(v => v.result.endpoint_id === value.endpoint_id);
                    assert.ok(endpoint);
                    if (verifications.has(value.operation_id)) assert.deepEqual(verifications.get(value.operation_id), value);
                    else {
                        assert.equal(endpoint.result.revision, value.expected_revision);
                        endpoint.result.verification_state = "verified"; endpoint.result.revision++;
                        verifications.set(value.operation_id, structuredClone(value));
                    }
                    if (lostVerification) { lostVerification = false; throw Error("fixture lost committed verification reply"); }
                    return { wait: async () => ({ snapshot: { state: "completed" }, value: structuredClone(endpoint.result) }) };
                }
                assert.equal(value.type, "identity");
                const old = identities.get(value.operation_id);
                if (old) assert.deepEqual(old.command, value);
                else identities.set(value.operation_id, { command: structuredClone(value), id: "id_" + identities.size });
                if (lostIdentity) { lostIdentity = false; throw Error("fixture lost committed identity response"); }
                return { wait: async () => ({ snapshot: { state: "completed" }, value: { kind: "identity", identity_id: identities.get(value.operation_id).id } }) };
            },
            close: async () => { adminCloses++; },
        };
    };
    const execute = async args => {
        calls.push({ cli: [...args] });
        const output = args[args.indexOf(args[0] === "issue" ? "--output" : "--installation") + 1];
        if (args[0] === "issue") {
            const request = JSON.parse(await readFile(args[args.indexOf("--request") + 1], "utf8"));
            assert.ok(!credentials.has(request.operation_id)); credentials.set(request.operation_id, request);
            await privateDirectory(output);
            await writeExact(join(output, "intent.json"), JSON.stringify(request));
            if (lostIssue) { lostIssue = false; throw Error("fixture lost installer reply"); }
        } else {
            assert.equal(args[0], "issue-resume");
            const request = JSON.parse(await readFile(join(output, "intent.json"), "utf8"));
            assert.deepEqual(credentials.get(request.operation_id), request);
        }
        const role = output.split("/").at(-1), identity = [...identities.values()].find(v => v.command.alias.endsWith("-" + role)).id;
        await writeExact(join(output, "client.toml"), 'credential = "fixture-child"\n' +
            'identity_id = "' + identity + '"\ndevice_id = "device_' + role + '"\n' +
            'device_kind = "desktop"\ndevice_platform = "connect-cli-issued"\nendpoint = "127.0.0.1:' + plan.ports.native + '"\n');
        await writeExact(join(output, "application.toml"), 'credential = "fixture-child"\n');
        await writeExact(join(output, "install.json"), '{"status":"fixture"}\n');
    };
    const createParticipantClient = path => ({ capabilities: { refresh: async () => {
        calls.push({ native: path });
        if (!nativeAllowed) throw Error("fixture current native access refused");
        if (changeDevice) {
            const native = join(path, "..", "client.toml"), bytes = await privateBytes(native);
            await writeFile(native, bytes.toString().replace('device_id = "device_owner"', 'device_id = "changed_device"'), { mode: 0o600 });
        }
        return { selectedTransport: "quic", features: new Set(["native-programmatic-media"]) };
    } }, close: async () => { participantCloses++; } });
    return { directory, plan, calls, identities, credentials, endpoints, verifications, createAdministrativeClient, execute,
        configure: () => configureActors({ directory, plan, execute, createAdministrativeClient, createParticipantClient }),
        loseIdentity: () => { lostIdentity = true; }, loseIssue: () => { lostIssue = true; },
        loseVerification: () => { lostVerification = true; }, refuseNative: () => { nativeAllowed = false; },
        changeNativeDeviceDuringRead: () => { changeDevice = true; },
        adminCloses: () => adminCloses, participantCloses: () => participantCloses };
}
test("four synthetic roles receive literal peer ceilings and public CLI-installed combined SDK configuration", async () => {
    const f = await fixture(); await verifyBootstrap(f.directory, true);
    const config = await f.configure(), original = await privateBytes(join(f.directory, "app.json"));
    assert.equal(f.identities.size, 4); assert.equal(f.credentials.size, 4);
    assert.deepEqual(Object.keys(config.actors), ["owner", "assistant", "booker", "organizer"]);
    for (const request of f.credentials.values()) {
        assert.deepEqual(request.operations, participantOperations);
        assert.equal(request.recipients.length, 3); assert.ok(!request.recipients.includes(request.identity_id));
        assert.ok(!request.operations.includes("admin") && !request.operations.includes("*") && !request.recipients.includes("*"));
        assert.equal(request.expires_at, f.plan.expiresAt);
    }
    for (const actor of Object.values(config.actors)) {
        assert.ok(actor.assets[actor.config]);
        const sdk = await privateBytes(join(f.directory, actor.config));
        assert.ok(sdk.toString().includes('[control]\ncredential = "fixture-child"'));
        assert.ok(sdk.toString().includes('[application]\ncredential = "fixture-child"'));
        assert.ok(!sdk.toString().includes("fixture-admin"));
    }
    await verifyBootstrap(f.directory, false); // operator.sdk.toml is derived, not an adopted bootstrap input.
    const again = await f.configure(); assert.deepEqual(again, config);
    assert.deepEqual(await privateBytes(join(f.directory, "app.json")), original);
    assert.equal(f.identities.size, 4); assert.equal(f.credentials.size, 4); assert.equal(f.adminCloses(), 4);
    assert.equal(f.calls.filter(value => value.cli?.[0] === "issue-resume").length, 4);
});
test("verified native media Endpoints bind original installed Devices and replay the same attributed decisions", async () => {
    const f = await fixture(), config = await f.configure();
    assert.equal(f.endpoints.size, 4); assert.equal(f.verifications.size, 4); assert.equal(f.participantCloses(), 4);
    for (const [role, actor] of Object.entries(config.actors)) {
        const endpoint = [...f.endpoints.values()].find(v => v.result.endpoint_id === actor.mediaEndpoint.id);
        assert.equal(endpoint.result.device_id, "device_" + role);
        assert.equal(endpoint.result.identity_id, actor.identityId);
        assert.deepEqual(endpoint.result.capabilities, ["audio", "data", "video"]);
        assert.equal(endpoint.result.verification_state, "verified");
        assert.equal(actor.mediaEndpoint.revision, 1);
        const command = JSON.parse(await privateBytes(join(f.directory, "requests", role + "-verify.json")));
        assert.equal(command.operation_id, f.plan.actors.find(v => v.role === role).verificationOperationId);
        assert.equal(command.expected_revision, 0);
        assert.match(command.evidence_ref, /^parley-native-health-sha256-[a-f0-9]{64}$/);
    }
    assert.deepEqual(await f.configure(), config);
    assert.equal(f.endpoints.size, 4); assert.equal(f.verifications.size, 4); assert.equal(f.participantCloses(), 8);
});
test("lost committed verification is recovered with original IDs and the original expected revision", async () => {
    const f = await fixture(); f.loseVerification();
    await assert.rejects(f.configure(), /lost committed verification/);
    const path = join(f.directory, "requests", "owner-verify.json"), original = await privateBytes(path);
    assert.equal(f.endpoints.size, 1); assert.equal(f.verifications.size, 1);
    const config = await f.configure();
    assert.deepEqual(await privateBytes(path), original);
    assert.equal(config.actors.owner.mediaEndpoint.revision, 1);
    assert.equal(f.endpoints.size, 4); assert.equal(f.verifications.size, 4);
});
test("current native access refusal creates no media Endpoints and closes both public client owners", async () => {
    const f = await fixture(); f.refuseNative();
    await assert.rejects(f.configure(), /current native access refused/);
    assert.equal(f.endpoints.size, 0); assert.equal(f.verifications.size, 0);
    assert.equal(f.participantCloses(), 1); assert.equal(f.adminCloses(), 2);
    await assert.rejects(privateBytes(join(f.directory, "app.json")), { code: "ENOENT" });
});
test("previous setup plans are refused unchanged rather than acquiring new setup IDs", async () => {
    const f = await fixture(), path = join(f.directory, "setup.json");
    for (const profile of ["parley-local-setup/1", "parley-local-setup/2"]) {
        const old = { ...f.plan, profile };
        await writeFile(path, JSON.stringify(old), { mode: 0o600 });
        const before = await privateBytes(path);
        await assert.rejects(loadOrCreatePlan(f.directory, binarySha, ports), { code: "app-setup-plan-refused" });
        assert.deepEqual(await privateBytes(path), before); assert.equal(f.calls.length, 0);
    }
});
test("a Device configuration changed during signed observation cannot authorize another Endpoint", async () => {
    const f = await fixture(); f.changeNativeDeviceDuringRead();
    await assert.rejects(f.configure(), { code: "app-native-device-refused" });
    assert.equal(f.endpoints.size, 0); assert.equal(f.verifications.size, 0);
    assert.equal(f.participantCloses(), 1); assert.equal(f.adminCloses(), 2);
});
test("a changed original verification revision is refused before another verification submission", async () => {
    const f = await fixture(); await f.configure();
    const path = join(f.directory, "requests", "owner-verify.json"), saved = JSON.parse(await privateBytes(path));
    saved.expected_revision++;
    await writeFile(path, JSON.stringify(saved, null, 2) + "\n", { mode: 0o600 });
    const count = f.calls.filter(v => v.command?.type === "verify_endpoint").length;
    await assert.rejects(f.configure(), { code: "app-native-verification-changed" });
    assert.equal(f.calls.filter(v => v.command?.type === "verify_endpoint").length, count);
    assert.equal(f.endpoints.size, 4); assert.equal(f.verifications.size, 4);
});
test("lost committed identity and interrupted child installation reuse original setup intent without new effects", async () => {
    const f = await fixture(); f.loseIdentity();
    await assert.rejects(f.configure(), /lost committed identity/);
    assert.equal(f.identities.size, 1); assert.equal(f.credentials.size, 0); assert.equal(f.adminCloses(), 1);
    const reopened = await loadOrCreatePlan(f.directory, binarySha, { native: 31111, http: 31112, ui: 31113 });
    assert.deepEqual(reopened, f.plan);
    f.loseIssue(); await assert.rejects(f.configure(), /lost installer/);
    assert.equal(f.identities.size, 4); assert.equal(f.credentials.size, 1);
    const original = await privateBytes(join(f.directory, "requests/owner.json"));
    await f.configure(); assert.equal(f.identities.size, 4); assert.equal(f.credentials.size, 4);
    assert.deepEqual(await privateBytes(join(f.directory, "requests/owner.json")), original);
    const first = f.calls.find(value => value.command).original;
    assert.equal(f.calls.filter(value => value.command && value.original === first).length, 3);
    assert.equal(f.calls.filter(value => value.cli?.[0] === "issue-resume").length, 1);
});
test("unsealed or changed bootstrap, changed runtime and linked private output refuse without replacement", async () => {
    const f = await fixture();
    await assert.rejects(verifyBootstrap(f.directory, false), { code: "app-bootstrap-incomplete" });
    await verifyBootstrap(f.directory, true);
    await writeFile(join(f.directory, "installation/server.toml"), "changed fixture provider configuration\n", { mode: 0o600 });
    await assert.rejects(verifyBootstrap(f.directory, false), { code: "app-setup-changed" });
    assert.equal(f.calls.length, 0);
    await assert.rejects(loadOrCreatePlan(f.directory, "b".repeat(64), ports), { code: "app-runtime-changed" });
    const original = await privateBytes(join(f.directory, "setup.json"));
    const linked = join(f.directory, "linked.json"); await symlink(join(f.directory, "setup.json"), linked);
    await assert.rejects(writeExact(linked, "replacement"), { code: "app-setup-file-refused" });
    assert.deepEqual(await privateBytes(join(f.directory, "setup.json")), original);
    assert.ok(!(await readdir(f.directory)).some(name => name.endsWith(".tmp")));
});
