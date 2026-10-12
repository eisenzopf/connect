/** Raw PCMU audio over a separate bounded binary helper channel. */
export declare class NativeAudioClient {
    private readonly child;
    private readonly timeoutMs;
    readonly publisherStreamId: string;
    private buffer;
    private readonly frames;
    private readonly waiters;
    private closed;
    private terminalError?;
    private constructor();
    static open(config: string, conversationId: string, sessionId: string, remoteIdentityId: string, executable?: string, timeoutMs?: number): Promise<NativeAudioClient>;
    private receiveBytes;
    private parseFrames;
    private fail;
    private write;
    subscribe(publisherStreamId: string): Promise<void>;
    sendPcmu(payload: Uint8Array): Promise<void>;
    receive(timeoutMs?: number): Promise<Uint8Array>;
    close(): Promise<void>;
}
