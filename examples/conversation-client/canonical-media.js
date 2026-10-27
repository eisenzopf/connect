import { BrowserClient } from "../../sdk/typescript/dist/browser.js";

/** Local test configuration. Private JWK/credential input stays in memory. */
export async function canonicalMediaClient(configuration, selection = {}) {
    configuration = structuredClone(configuration);
    const connection = configuration.connection && {...configuration.connection};
    if (!connection || !configuration.end ||
        (selection.conversationId !== undefined && selection.conversationId !== connection.conversationId) ||
        (selection.sessionId !== undefined && selection.sessionId !== connection.sessionId) ||
        (selection.identityId !== undefined && selection.identityId !== configuration.identityId))
        throw Error("Select the signed media configuration for this identity, Conversation and accepted Session.");
    const parameters = (algorithm) => {
        if (algorithm === "EdDSA") return {name: "Ed25519"};
        if (algorithm === "ES256") return {name: "ECDSA", namedCurve: "P-256"};
        throw Error("Use an enrolled EdDSA or ES256 signing identity.");
    };
    const signing = configuration.signing, server = configuration.server;
    const [key, trust] = await Promise.all([
        crypto.subtle.importKey("jwk", signing.jwk, parameters(signing.algorithm), false, ["sign"]),
        crypto.subtle.importKey("jwk", server.jwk, parameters(server.algorithm), false, ["verify"]),
    ]);
    const options = Object.fromEntries([
        "conversationId", "sessionId", "connectionId", "operationId", "expectedRevision",
        "participantId", "audioStreamId", "videoStreamId",
    ].filter(name => connection[name] !== undefined).map(name => [name, connection[name]]));
    const end = {operationId: configuration.end.operationId, expectedRevision: configuration.end.expectedRevision};
    if (typeof end.operationId !== "string" || !end.operationId || end.operationId === options.operationId ||
        !Number.isSafeInteger(end.expectedRevision) || end.expectedRevision < 0)
        throw Error("Supply a distinct original end operation and its expected revision.");
    const client = await BrowserClient.open({
        transport: "websocket", url: configuration.url, credential: configuration.credential,
        identityId: configuration.identityId, device: configuration.device,
        signing: {algorithm: signing.algorithm, keyId: signing.keyId, key},
        server: {algorithm: server.algorithm, keyId: server.keyId, key: trust, notBefore: server.notBefore, expiresAt: server.expiresAt},
    });
    const receiveSelection = configuration.subscribe && structuredClone(configuration.subscribe);
    const scope = {cid: options.conversationId, sid: options.sessionId, connid: options.connectionId};
    let receiving;
    return {client, options, end, url: configuration.url, receive() {
        if (receiving) return receiving;
        if (!receiveSelection || !/^[A-Za-z0-9._~-]{1,128}$/.test(receiveSelection.operationId) ||
            receiveSelection.operationId === options.operationId || receiveSelection.operationId === end.operationId ||
            !Number.isSafeInteger(receiveSelection.expectedRevision) || receiveSelection.expectedRevision < 0 || !Array.isArray(receiveSelection.subscriptions))
            return Promise.reject(Error("Supply an explicit original receive-receiveSelection operation, revision and subscriptions."));
        receiving = (async()=>{
            const until = performance.now() + 10000;
            await client.request("stream.subscribe", {subscriptions: receiveSelection.subscriptions},
                {...scope, operation_id: receiveSelection.operationId, expected_revision: receiveSelection.expectedRevision});
            for (;;) {
                const state=await client.operationStatus(scope.cid,receiveSelection.operationId,{timeoutMs:Math.max(1,until-performance.now())});
                if(state.state === "completed") return;
                if(["failed","cancelled"].includes(state.state) || performance.now() >= until)
                    throw Error("Original receive receiveSelection is not confirmed complete; inspect its operation status.");
                await new Promise(resolve=>setTimeout(resolve,20));
            }
        })();
        return receiving;
    }};
}
export async function readCanonicalMedia(file, selection) {
    if (!file || file.size > 65536) throw Error("Choose a bounded signed media configuration file first.");
    return canonicalMediaClient(JSON.parse(await file.text()), selection);
}
