// Actual native-host observations; no declared topology is accepted as evidence.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile, readdir, readlink, stat } from "node:fs/promises";
import { isAbsolute } from "node:path";
import { release } from "node:os";

const hash = value => createHash("sha256").update(value).digest("hex");
export function nativeHost() {
    assert.ok((process.platform === "linux" && process.arch === "x64") ||
        (process.platform === "darwin" && process.arch === "arm64"));
    return { platform: process.platform === "linux" ? "ubuntu24-x86_64" : "macos-arm64",
        os: process.platform, architecture: process.arch, kernel_release: release() };
}
export async function executableHash(pid) {
    assert.ok(Number.isSafeInteger(pid) && pid > 0);
    if (process.platform === "linux") return hash(await readFile(`/proc/${pid}/exe`));
    assert.equal(process.platform, "darwin");
    let path;
    try {
        path = execFileSync("/bin/ps", ["-ww", "-p", String(pid), "-o", "comm="], { encoding: "utf8", timeout: 5000 }).trim();
    } catch (error) {
        // The scanner itself can appear in the child snapshot and exit before
        // image lookup. Only ps's empty "no matching PID" result is absence.
        if (error.status === 1 && !String(error.stdout).trim() && !String(error.stderr).trim())
            throw Object.assign(Error("Native process exited"), { code: "ESRCH" });
        throw error;
    }
    assert.ok(isAbsolute(path));
    return hash(await readFile(path));
}
export function childPids(parent) {
    const rows = execFileSync("ps", ["-axo", "pid=,ppid="], { encoding: "utf8", timeout: 5000 }).trim().split("\n");
    return rows.map(line => {
        const match = /^\s*(\d+)\s+(\d+)\s*$/.exec(line);
        assert.ok(match); return [Number(match[1]), Number(match[2])];
    }).filter(([, ppid]) => ppid === parent).map(([pid]) => pid);
}
export async function processExists(pid) {
    try {
        if (process.platform === "linux") await stat(`/proc/${pid}`);
        else process.kill(pid, 0);
        return true;
    } catch (error) {
        if (["ENOENT", "ESRCH"].includes(error.code)) return false;
        throw error;
    }
}
export async function observeLocalTopology(server, helpers, endpoint, expectedBinary) {
    const host = nativeHost();
    assert.equal(await executableHash(server), expectedBinary);
    assert.equal(helpers.size, 5);
    for (const pid of helpers) assert.equal(await executableHash(pid), expectedBinary);
    assert.match(endpoint, /^127\.0\.0\.1:[1-9][0-9]{0,4}$/);
    const port = Number(endpoint.split(":")[1]); assert.ok(port <= 65535);
    if (process.platform === "linux") {
        const namespace = await readlink(`/proc/${server}/ns/net`);
        for (const pid of helpers) assert.equal(await readlink(`/proc/${pid}/ns/net`), namespace);
        const inodes = new Set();
        for (const fd of await readdir(`/proc/${server}/fd`)) {
            try {
                const value = await readlink(`/proc/${server}/fd/${fd}`), match = /^socket:\[(\d+)\]$/.exec(value);
                if (match) inodes.add(match[1]);
            } catch (error) { if (error.code !== "ENOENT") throw error; }
        }
        const udp = (await readFile(`/proc/${server}/net/udp`, "utf8")).trim().split("\n").slice(1).map(line => line.trim().split(/\s+/));
        assert.ok(udp.some(row => row[1].split(":")[0] === "0100007F" && parseInt(row[1].split(":")[1], 16) === port && inodes.has(row[9])));
        return { host, same_network_namespace_observed: true, server_owned_loopback_udp_bind_observed: true,
            helpers_in_server_namespace: helpers.size, server_executable_sha256: expectedBinary, namespace_sha256: hash(namespace) };
    }
    const sockets = execFileSync("/usr/sbin/lsof", ["-nP", "-a", "-p", String(server), "-iUDP@" + endpoint, "-Fpn"],
        { encoding: "utf8", timeout: 5000 }).trim().split("\n");
    assert.equal(sockets[0], "p" + server);
    assert.ok(sockets.includes("n" + endpoint));
    const boot = execFileSync("/usr/sbin/sysctl", ["-n", "kern.bootsessionuuid"], { encoding: "utf8", timeout: 5000 }).trim();
    assert.match(boot, /^[0-9A-Fa-f-]{36}$/);
    return { host, server_owned_loopback_udp_bind_observed: true, helpers_observed_on_native_host: helpers.size,
        server_executable_sha256: expectedBinary, native_kernel_identity_sha256: hash(boot) };
}
