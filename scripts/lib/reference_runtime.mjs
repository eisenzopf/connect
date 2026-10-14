/** Disposable local runtime owned by the reference UI qualification process. */
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { createServer } from "node:net";
import { once } from "node:events";
import { spawn, execFileSync } from "node:child_process";

async function port() {
    const reserve = createServer();
    reserve.listen(0, "127.0.0.1");
    await once(reserve, "listening");
    const selected = reserve.address().port;
    await new Promise((resolve) => reserve.close(resolve));
    return selected;
}

export class ReferenceRuntime {
    constructor(executable, scratch, environment) {
        this.executable = executable;
        this.scratch = scratch;
        this.environment = environment;
        this.state = path.join(scratch, "state");
        this.run([
            "init",
            "--directory",
            this.state,
            "--example",
            "review-loop",
            "--bind",
            "127.0.0.1:0",
        ]);
        const configurations = ["reviews", "agent"].map((name) =>
            this.load(name),
        );
        for (const [index, name] of ["reviews", "agent"].entries()) {
            const output = path.join(this.state, name + "-av.toml");
            this.run([
                "issue",
                "--config",
                path.join(this.state, "operator.toml"),
                "--server-config",
                path.join(this.state, "server.toml"),
                "--identity-id",
                configurations[index].identity_id,
                "--recipients",
                configurations[1 - index].identity_id,
                "--voice",
                "--output",
                output,
            ]);
            fs.renameSync(output, path.join(this.state, name + ".toml"));
        }
        this.config = this.load("reviews");
        this.peer = this.load("agent");
        this.log = fs.openSync(path.join(scratch, "server.log"), "w", 0o600);
    }
    run(arguments_) {
        return execFileSync(this.executable, arguments_, {
            stdio: "pipe",
            env: this.environment,
            timeout: 15000,
        });
    }
    load(name) {
        return JSON.parse(
            execFileSync(
                "python3",
                [
                    "-c",
                    'import json,tomllib,sys;print(json.dumps(tomllib.load(open(sys.argv[1],"rb"))))',
                    path.join(this.state, name + ".toml"),
                ],
                { encoding: "utf8", env: this.environment, timeout: 10000 },
            ),
        );
    }
    async start() {
        this.httpPort ??= await port();
        this.child = spawn(
            this.executable,
            [
                "serve",
                "--config",
                path.join(this.state, "server.toml"),
                "--http-bind",
                `127.0.0.1:${this.httpPort}`,
            ],
            { stdio: ["ignore", this.log, this.log], env: this.environment },
        );
        this.child.on("error", () => {
            this.startupFailed = true;
        });
        return `http://127.0.0.1:${this.httpPort}`;
    }
    async ready(client) {
        for (let attempt = 0; attempt < 100; attempt++) {
            if (this.startupFailed || this.child.exitCode !== null)
                throw Error("owned server failed at startup");
            try {
                await client.health();
                return;
            } catch {
                await new Promise((resolve) => setTimeout(resolve, 100));
            }
        }
        throw Error("owned server did not become ready");
    }
    async stop() {
        const child = this.child;
        if (!child) return;
        if (child.exitCode === null) {
            child.kill("SIGINT");
            await Promise.race([
                once(child, "exit"),
                new Promise((resolve) => setTimeout(resolve, 5000)),
            ]);
            if (child.exitCode === null) {
                child.kill("SIGKILL");
                await once(child, "exit");
                throw Error("owned server did not stop within five seconds");
            }
        }
        assert.equal(child.exitCode, 0, "owned server exited unsuccessfully");
    }
    async enableVideo(conversation, client) {
        await this.stop();
        this.signalingPort = await port();
        fs.appendFileSync(
            path.join(this.state, "server.toml"),
            `\n[webrtc]\nbind = "127.0.0.1:${this.signalingPort}"\n[webrtc.route]\n` +
                `credential_id = ${JSON.stringify(this.config.credential_id)}\nconversation_id = ${JSON.stringify(conversation)}\n` +
                `assistant_identity_id = ${JSON.stringify(this.peer.identity_id)}\n`,
        );
        await this.start();
        await this.ready(client);
    }
    async close() {
        try {
            await this.stop();
        } finally {
            fs.closeSync(this.log);
        }
    }
}
