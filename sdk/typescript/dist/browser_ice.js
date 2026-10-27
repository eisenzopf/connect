import { ConnectError } from "./errors.js";
/** Snapshot explicit ICE services before capture. No implicit public service. */
export function browserIceConfiguration(servers) {
    if (servers === undefined)
        return undefined;
    const invalid = () => new ConnectError("invalid-request", "Invalid bounded explicit ICE server configuration");
    if (!Array.isArray(servers) || servers.length > 8)
        throw invalid();
    let count = 0;
    const iceServers = Array.from(servers, (server) => {
        if (!server ||
            typeof server !== "object" ||
            Array.isArray(server) ||
            Object.keys(server).some((key) => !["urls", "username", "credential"].includes(key)))
            throw invalid();
        const urls = typeof server.urls === "string" ? [server.urls] : server.urls;
        if (!Array.isArray(urls) ||
            !urls.length ||
            urls.length > 4 ||
            (count += urls.length) > 16)
            throw invalid();
        const copied = Array.from(urls, (value) => {
            if (typeof value !== "string" || value.length > 2048)
                throw invalid();
            const match = /^(stun|stuns|turn|turns):(\[[A-Fa-f0-9:.]+\]|[A-Za-z0-9.-]+)(?::([0-9]{1,5}))?(?:\?transport=(udp|tcp))?$/.exec(value);
            if (!match ||
                (match[1].startsWith("stun") && match[4]) ||
                (match[3] && (Number(match[3]) < 1 || Number(match[3]) > 65535)))
                throw invalid();
            try {
                new URL(`https://${match[2]}${match[3] ? ":" + match[3] : ""}`);
            }
            catch {
                throw invalid();
            }
            if (match[1].startsWith("turn") &&
                (!server.username || !server.credential))
                throw invalid();
            return value;
        });
        if ((server.username !== undefined &&
            (typeof server.username !== "string" ||
                server.username.length > 512)) ||
            (server.credential !== undefined &&
                (typeof server.credential !== "string" ||
                    server.credential.length > 2048)))
            throw invalid();
        return {
            urls: copied,
            ...(server.username === undefined
                ? {}
                : { username: server.username }),
            ...(server.credential === undefined
                ? {}
                : { credential: server.credential }),
        };
    });
    return { iceServers };
}
