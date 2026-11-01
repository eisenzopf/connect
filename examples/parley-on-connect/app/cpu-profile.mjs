// Explicit rehearsal diagnostics only. No inspector listener or remote session.
// Raw profiles can contain private source paths and stay under ignored state.
import { Session } from "node:inspector";
import { lstat, realpath, writeFile } from "node:fs/promises";
import { join, resolve, sep } from "node:path";
import { randomUUID } from "node:crypto";

export async function startCpuProfile(directory, maximumMs = 120000) {
    const path = resolve(directory), info = await lstat(path);
    if (path !== directory || !path.split(sep).includes(".local") || !info.isDirectory() ||
        info.isSymbolicLink() || info.mode & 0o077 || await realpath(path) !== path ||
        !Number.isInteger(maximumMs) || maximumMs < 10 || maximumMs > 120000)
        throw Error("Private bounded CPU profile required");
    const session = new Session(); session.connect();
    const post = (method, params = {}) => new Promise((done, reject) =>
        session.post(method, params, (error, result) => error ? reject(error) : done(result)));
    try {
        await post("Profiler.enable");
        await post("Profiler.setSamplingInterval", { interval: 1000 });
        await post("Profiler.start");
    } catch (error) { session.disconnect(); throw error; }
    let stopping, timer;
    const stop = () => stopping ??= (async () => {
        clearTimeout(timer);
        try {
            const { profile } = await post("Profiler.stop");
            if (!Array.isArray(profile.nodes) || profile.nodes.length > 50000 ||
                !Array.isArray(profile.samples) || profile.samples.length > 150000)
                throw Error("CPU profile observation capacity");
            const bytes = Buffer.from(JSON.stringify(profile));
            if (bytes.length > 8 * 1024 * 1024) throw Error("CPU profile byte capacity");
            const file = join(path, "cpu-" + randomUUID() + ".json");
            await writeFile(file, bytes, { flag: "wx", mode: 0o600 });
            return { file, samples: profile.samples.length, nodes: profile.nodes.length, bytes: bytes.length };
        } finally { session.disconnect(); }
    })();
    // A later close observes any failure. Never leave an unhandled timer error.
    timer = setTimeout(() => { void stop().catch(() => {}); }, maximumMs); timer.unref();
    return Object.freeze({ stop });
}
