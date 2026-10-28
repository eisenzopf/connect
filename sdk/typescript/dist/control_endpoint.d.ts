/** A configured destination is captured once; redirects never change it. */
export type HttpControlTransport = "loopback-http" | "remote-https";
export declare function controlEndpoint(endpoint: string, credential: string, transport?: HttpControlTransport): string;
