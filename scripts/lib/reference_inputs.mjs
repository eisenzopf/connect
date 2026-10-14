/** Bind only public code/tool inputs. Private state, logs and credentials are excluded. */
import fs from "node:fs";
import path from "node:path";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";

export function referenceInputSnapshot(assets, executable, environment) {
    const inputs = [
        executable,
        process.execPath,
        fileURLToPath(
            new URL("../check-reference-client.mjs", import.meta.url),
        ),
        fileURLToPath(import.meta.url),
        fileURLToPath(new URL("./reference_runtime.mjs", import.meta.url)),
        fileURLToPath(new URL("./reference_video.mjs", import.meta.url)),
    ];
    for (const relative of [
        "examples/conversation-client",
        "sdk/typescript/dist",
    ])
        for (const entry of fs.readdirSync(path.join(assets, relative), {
            withFileTypes: true,
        }))
            if (entry.isFile() && /\.(js|html|css|ts)$/.test(entry.name))
                inputs.push(path.join(assets, relative, entry.name));
    for (const name of [
        "CONNECT_CHROME_BINARY",
        "CONNECT_CHROME_BIN",
        "CONNECT_PLAYWRIGHT_MODULE",
    ])
        if (environment[name]) {
            const selected = path.resolve(environment[name]);
            if (!fs.statSync(selected).isFile()) throw Error("Select an ordinary tool/module entry file");
            inputs.push(selected);
            if (name === "CONNECT_PLAYWRIGHT_MODULE") {
                const manifest = path.join(path.dirname(selected), "package.json");
                if (fs.existsSync(manifest) && fs.statSync(manifest).isFile()) inputs.push(manifest);
            }
        }
    return Object.fromEntries(
        [...new Set(inputs)]
            .sort()
            .map((input) => [
                input,
                createHash("sha256")
                    .update(fs.readFileSync(input))
                    .digest("hex"),
            ]),
    );
}
