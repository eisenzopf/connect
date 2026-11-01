// Application-owned setup. Connect state is created only by the installed CLI
// and public trusted SDK. This module never opens the Connect database.
import { randomUUID, createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { open, readFile, lstat, realpath, mkdir, link, unlink, readdir } from "node:fs/promises";
import { join, resolve, sep } from "node:path";

const fail = code => { throw Object.assign(Error(code), { code }); };
const id = value => typeof value === "string" && /^[A-Za-z0-9._~-]{1,128}$/.test(value);
export const actorDefinitions = Object.freeze([
    Object.freeze({ role: "owner", label: "Owner", kind: "human" }),
    Object.freeze({ role: "assistant", label: "Assistant (simulated)", kind: "ai" }),
    Object.freeze({ role: "booker", label: "Booker", kind: "human" }),
    Object.freeze({ role: "organizer", label: "Organizer", kind: "human" }),
]);
export const participantOperations = Object.freeze([
    "resolve", "create", "conversation.list", "conversation.get", "conversation.subscribe",
    "send", "history", "inbox", "operation.get", "health", "session_get", "session.observation",
    "session.invite", "session.accept", "session.reject", "session.cancel", "session.end", "session.update",
    "connection.offer", "connection.update", "capability.advertise",
].sort());
export async function digest(path) {
    const hash = createHash("sha256"); for await (const block of createReadStream(path)) hash.update(block); return hash.digest("hex");
}
export async function privateDirectory(path) {
    await mkdir(path, { recursive: true, mode: 0o700 });
    const info = await lstat(path);
    if (!info.isDirectory() || info.isSymbolicLink() || (info.mode & 0o077) || await realpath(path) !== path ||
        !resolve(path).split(sep).includes(".local")) fail("private-app-state-required");
}
export async function privateBytes(path) {
    const info = await lstat(path);
    if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1 || (info.mode & 0o077) ||
        info.size > 65536 || await realpath(path) !== path) fail("app-setup-file-refused");
    return readFile(path);
}
async function optionalBytes(path) {
    try { return await privateBytes(path); } catch (error) { if (error.code === "ENOENT") return null; throw error; }
}
const encode = value => Buffer.from(JSON.stringify(value, null, 2) + "\n");
/** Publish completely synced bytes without replacing existing state. Reopening
 * accepts only the same bytes, including after concurrent no-replace publication. */
export async function writeExact(path, bytes) {
    bytes = Buffer.from(bytes);
    if (bytes.length > 65536) fail("app-setup-file-refused");
    const saved = await optionalBytes(path);
    if (saved) { if (!saved.equals(bytes)) fail("app-setup-changed"); return; }
    const temporary = path + "." + randomUUID() + ".tmp", file = await open(temporary, "wx", 0o600);
    try { await file.writeFile(bytes); await file.sync(); } finally { await file.close(); }
    try {
        try { await link(temporary, path); }
        catch (error) { if (error.code !== "EEXIST") throw error; if (!(await readFile(path)).equals(bytes)) fail("app-setup-changed"); }
    } finally { await unlink(temporary); }
    const directory = await open(resolve(path, ".."), "r"); try { await directory.sync(); } finally { await directory.close(); }
    if (!(await privateBytes(path)).equals(bytes)) fail("app-setup-changed");
}
export async function verifyBootstrap(directory, newlyInitialized) {
    const installation = join(directory, "installation"), files = {};
    for (const name of (await readdir(installation)).sort()) {
        if (!/^[a-z0-9.-]+\.(?:toml|der|pub)$/.test(name) || name === "operator.sdk.toml") continue;
        const bytes = await privateBytes(join(installation, name));
        files[name] = createHash("sha256").update(bytes).digest("hex");
    }
    for (const name of ["server.toml", "operator.toml", "operator.application.toml", "certificate.der", "private-key.der", "server-signing.der", "server-signing.pub"])
        if (!files[name]) fail("app-bootstrap-incomplete");
    const path = join(directory, "bootstrap.json");
    // An initializer that exited before its controller sealed the result needs
    // inspection, rather than adoption of an unproven existing configuration.
    if (!newlyInitialized && !await optionalBytes(path)) fail("app-bootstrap-incomplete");
    await writeExact(path, encode({ profile: "parley-local-bootstrap/1", files }));
}
export function newSetupPlan(binarySha256, ports) {
    const prefix = "parley_" + randomUUID().replaceAll("-", "");
    return { profile: "parley-local-setup/1", sdkVersion: "0.1.0", binarySha256, ports,
        expiresAt: Math.floor(Date.now() / 1000) + 86400,
        actors: actorDefinitions.map(actor => ({ ...actor, alias: prefix + "_" + actor.role,
            identityOperationId: "identity_" + randomUUID(), credentialOperationId: "credential_" + randomUUID() })) };
}
function validatePlan(plan) {
    if (!plan || plan.profile !== "parley-local-setup/1" || plan.sdkVersion !== "0.1.0" ||
        !/^[a-f0-9]{64}$/.test(plan.binarySha256) || !Number.isSafeInteger(plan.expiresAt) ||
        plan.expiresAt <= Math.floor(Date.now() / 1000) || !plan.ports || Object.keys(plan.ports).sort().join() !== "http,native,ui" ||
        Object.values(plan.ports).some(value => !Number.isSafeInteger(value) || value < 1024 || value > 65535) ||
        plan.ports.http === plan.ports.ui || !Array.isArray(plan.actors) || plan.actors.length !== 4) fail("app-setup-plan-refused");
    const originals = new Set();
    for (const [index, actor] of plan.actors.entries()) {
        const expected = actorDefinitions[index];
        if (actor.role !== expected.role || actor.label !== expected.label || actor.kind !== expected.kind ||
            !id(actor.alias) || !id(actor.identityOperationId) || !id(actor.credentialOperationId)) fail("app-setup-plan-refused");
        for (const value of [actor.alias, actor.identityOperationId, actor.credentialOperationId]) {
            if (originals.has(value)) fail("app-setup-plan-refused"); originals.add(value);
        }
    }
}
export async function loadOrCreatePlan(directory, binarySha256, ports) {
    const path = join(directory, "setup.json"), saved = await optionalBytes(path);
    const plan = saved ? JSON.parse(saved) : newSetupPlan(binarySha256, ports);
    validatePlan(plan);
    if (plan.binarySha256 !== binarySha256) fail("app-runtime-changed");
    if (!saved) await writeExact(path, encode(plan));
    return plan;
}
export async function combineSdkConfig(directory, nativeFile, applicationFile, output) {
    const native = await privateBytes(join(directory, nativeFile));
    const application = await privateBytes(join(directory, applicationFile));
    // Both tables are the CLI's validated TOML, kept in their original directory
    // so public SDK relative trust/key resolution remains correct.
    await writeExact(join(directory, output), Buffer.concat([Buffer.from('tenant_id = "tnt_default"\n[control]\n'),
        native, Buffer.from("\n[application]\n"), application]));
}
/** `execute` runs the installed public CLI with bounded private output. The SDK
 * factory is supplied by run.mjs's ordinary installed-package import. */
export async function configureActors({ directory, plan, execute, createAdministrativeClient, signal }) {
    validatePlan(plan);
    const installation = join(directory, "installation");
    await combineSdkConfig(installation, "operator.toml", "operator.application.toml", "operator.sdk.toml");
    const admin = createAdministrativeClient(join(installation, "operator.sdk.toml")), identities = {};
    const options = { timeoutMs: 8000, ...(signal ? { signal } : {}) };
    try {
        for (const actor of plan.actors) {
            signal?.throwIfAborted();
            const command = admin.validateProvisioning({ type: "identity", operation_id: actor.identityOperationId,
                alias: actor.alias, kind: actor.kind, display_name: actor.label });
            // A rerun explicitly replays the original immutable local issuance
            // under current administrator authority; it never invents a new ID.
            const operation = await admin.provision(command, { ...options, operationId: actor.identityOperationId });
            const result = await operation.wait(options);
            if (result.snapshot.state !== "completed" || result.value?.kind !== "identity" || !id(result.value.identity_id)) fail("app-identity-unobserved");
            identities[actor.role] = result.value.identity_id;
        }
    } finally { await admin.close({ timeoutMs: 5000 }); }
    if (new Set(Object.values(identities)).size !== 4) fail("app-identity-unobserved");
    await privateDirectory(join(directory, "actors")); await privateDirectory(join(directory, "requests"));
    const actors = {};
    for (const actor of plan.actors) {
        signal?.throwIfAborted();
        const request = { operation_id: actor.credentialOperationId, identity_id: identities[actor.role],
            operations: participantOperations, recipients: Object.entries(identities).filter(([role]) => role !== actor.role).map(([, identity]) => identity).sort(),
            expires_at: plan.expiresAt };
        const requestPath = join(directory, "requests", actor.role + ".json"), output = join(directory, "actors", actor.role);
        await writeExact(requestPath, encode(request));
        // Existing or partial installations are reconciled only by the original
        // public installer. Never inspect, rewrite or repair its private intent.
        let exists = false;
        try { await lstat(output); exists = true; } catch (error) { if (error.code !== "ENOENT") throw error; }
        await execute(exists ? ["issue-resume", "--config", join(installation, "operator.application.toml"), "--installation", output] :
            ["issue", "--config", join(installation, "operator.application.toml"), "--native-template", join(installation, "operator.toml"),
                "--request", requestPath, "--output", output, "--signing-algorithm", "EdDSA"]);
        await privateDirectory(output);
        await combineSdkConfig(output, "client.toml", "application.toml", "sdk.toml");
        const assets = {};
        for (const name of (await readdir(output)).sort()) {
            if (!/^[a-z0-9.-]{1,128}$/.test(name)) fail("app-setup-file-refused");
            const bytes = await privateBytes(join(output, name));
            assets["actors/" + actor.role + "/" + name] = createHash("sha256").update(bytes).digest("hex");
        }
        actors[actor.role] = { label: actor.label, identityId: identities[actor.role], config: "actors/" + actor.role + "/sdk.toml", assets };
    }
    const config = { profile: "parley-local-app/1", mode: "fixture", sdkVersion: plan.sdkVersion, port: plan.ports.ui,
        runtime: { executable: "runtime/connect", sha256: plan.binarySha256 }, actors };
    await writeExact(join(directory, "app.json"), encode(config));
    return config;
}
