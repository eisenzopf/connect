/** A configured destination is captured once; redirects never change it. */
export type HttpControlTransport = "loopback-http" | "remote-https";
export function controlEndpoint(
    endpoint: string,
    credential: string,
    transport: HttpControlTransport = "loopback-http",
): string {
    let url: URL;
    try {
        if (typeof endpoint !== "string" || endpoint.length > 4096)
            throw new Error();
        url = new URL(endpoint);
    } catch {
        throw new Error("Invalid explicit HTTP control configuration");
    }
    if (
        (transport !== "loopback-http" && transport !== "remote-https") ||
        (transport === "loopback-http"
            ? url.protocol !== "http:" ||
              !["127.0.0.1", "[::1]", "localhost"].includes(url.hostname)
            : url.protocol !== "https:") ||
        url.username ||
        url.password ||
        url.search ||
        url.hash ||
        (url.pathname !== "/" && url.pathname !== "") ||
        typeof credential !== "string" ||
        !credential ||
        credential.length > 16_384 ||
        /[\x00-\x20\x7f]/.test(credential)
    )
        throw new Error("Invalid explicit HTTP control configuration");
    return url.origin;
}
