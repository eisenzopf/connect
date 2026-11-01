// Closed backend diagnostics. Never serialize exception messages, arbitrary
// properties, command inputs, credentials, resource IDs or full filesystem paths.
export function appDiagnostic(error) {
    const kinds = ["validation", "configuration", "authentication", "authorization", "deadline", "cancelled",
        "protocol", "remote-error", "transport-uncertain", "unsupported", "conflict", "resync-required"];
    const local = ["invalid-app-intent", "different-operation-observation", "stale-operation-observation",
        "different-operation-result", "original-operation-needs-recovery", "invalid-app-transition", "corrupt-app-state"];
    const code = typeof error?.code === "string" && (/^app-[a-z-]{1,48}$/.test(error.code) || local.includes(error.code)) ? error.code : null;
    const location = /\/app\/([a-z_]+\.mjs):(\d+):(\d+)/.exec(String(error?.stack));
    const sdk = /\/node_modules\/@rudeless\/connect\/dist\/(node_connections|node_media|node_sdk|sdk_helper|node_receipts|canonical_http|node_media_channel)\.js:(\d+):(\d+)/.exec(String(error?.stack));
    return { code, name: ["Error", "TypeError", "RangeError", "NodeSdkError"].includes(error?.name) ? error.name : null,
        kind: kinds.includes(error?.kind) ? error.kind : null,
        location: location ? { file: location[1], line: Number(location[2]), column: Number(location[3]) } : null,
        sdk_location: sdk ? { file: sdk[1] + ".js", line: Number(sdk[2]), column: Number(sdk[3]) } : null };
}
