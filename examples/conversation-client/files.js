// Artifact controls share the public client and durable intent journal.
import {
    act,
    mutate,
    needClient,
    needConversation,
    refresh,
    journal,
} from "./app.js";
import { readArtifactBytes } from "./workflow.js";
const fileElement = (id) => document.getElementById(id);

export function renderArtifactHistory(items) {
    const latest = new Map();
    for (const item of items) {
        if (!item.kind.startsWith("artifact_")) continue;
        try {
            const artifact = JSON.parse(item.body);
            latest.set(artifact.id, artifact);
        } catch {
            continue;
        } // An unsupported historical projection is still in the timeline.
    }
    const list = fileElement("artifact-list");
    list.replaceChildren();
    for (const artifact of latest.values()) {
        const row = document.createElement("li");
        const label = document.createElement("span");
        label.textContent = `${artifact.name} · ${artifact.size} bytes · ${artifact.state}`;
        const select = document.createElement("button");
        select.textContent = "Select file";
        select.onclick = () => {
            fileElement("artifact-id").value = artifact.id;
        };
        row.append(label, select);
        list.append(row);
    }
}

fileElement("upload").onclick = () =>
    act("File uploaded", async () => {
        const file = fileElement("file").files[0];
        if (!file)
            throw new Error("Select the original file to start or resume.");
        if (file.size > 16 * 1024 * 1024)
            throw new Error("File exceeds the 16 MiB local profile.");
        const cid = needConversation();
        const bytes = new Uint8Array(await file.arrayBuffer());
        const digest = Array.from(
            new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
        )
            .map((b) => b.toString(16).padStart(2, "0"))
            .join("");
        const prefix = `upload:${cid}`;
        const ttl = Number(fileElement("file-ttl").value || 0);
        if (!Number.isInteger(ttl) || ttl < 0 || ttl > 604800)
            throw new Error(
                "Choose an expiry from 1 to 604800 seconds, or leave it empty.",
            );
        const pending = journal().read()[`${prefix}:begin`];
        const expires_at = pending
            ? (pending.operation.expires_at ?? null)
            : ttl
              ? Math.floor(Date.now() / 1000) + ttl
              : null;
        const artifact = await mutate(
            `${prefix}:begin`,
            {
                op: "artifact_begin",
                conversation_id: cid,
                name: file.name,
                media_type: file.type || "application/octet-stream",
                size: bytes.length,
                sha256: digest,
                expires_at,
            },
            true,
        );
        for (
            let offset = journal().progress(prefix);
            offset < bytes.length;
            offset += 32768
        ) {
            const chunk = bytes.slice(offset, offset + 32768);
            const data = btoa(String.fromCharCode(...chunk));
            // Save one chunk intent; advance local progress only after its receipt is saved.
            await mutate(
                `${prefix}:${offset}`,
                {
                    op: "artifact_append",
                    conversation_id: cid,
                    artifact_id: artifact.id,
                    offset,
                    data,
                },
                true,
            );
            journal().saveProgress(prefix, offset + chunk.length);
            journal().forget(`${prefix}:${offset}`);
            fileElement("file-state").textContent =
                `${Math.min(offset + chunk.length, bytes.length)} / ${bytes.length} bytes`;
        }
        await mutate(
            `${prefix}:finish`,
            {
                op: "artifact_finish",
                conversation_id: cid,
                artifact_id: artifact.id,
            },
            true,
        );
        for (const slot of Object.keys(journal().read()).filter((k) =>
            k.startsWith(prefix + ":"),
        ))
            journal().forget(slot);
        journal().saveProgress(prefix, 0);
        fileElement("file-state").textContent = `Ready · ${artifact.id}`;
        await refresh();
    });

fileElement("download").onclick = () =>
    act("File verified", async () => {
        const conversation = needConversation(),
            id = fileElement("artifact-id").value.trim();
        if (!id) throw new Error("Select a file or enter its artifact ID.");
        const { artifact, bytes } = await readArtifactBytes(
            needClient(),
            conversation,
            id,
        );
        if (conversation !== needConversation())
            throw new Error(
                "The selected conversation changed during download.",
            );
        const url = URL.createObjectURL(
            new Blob([bytes], { type: "application/octet-stream" }),
        );
        try {
            const anchor = document.createElement("a");
            anchor.href = url;
            anchor.download =
                artifact.name
                    .split(/[\/\\]/)
                    .pop()
                    .replace(/[\x00-\x1f\x7f]/g, "") || "connect-file";
            anchor.click();
        } finally {
            URL.revokeObjectURL(url);
        }
        fileElement("file-state").textContent =
            `Verified ${bytes.length} bytes · SHA-256 ${artifact.sha256}`;
    });
